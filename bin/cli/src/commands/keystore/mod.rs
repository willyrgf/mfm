use clap::Subcommand;

use super::CommandContext;

/// Delete-key command implementation.
mod delete;
/// Import-key command implementation.
mod import;
/// List-keys command implementation.
mod list;
/// Transaction signing command implementation.
mod tx_sign;

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
    /// Sign an EIP-1559 transaction payload using a keystore-managed key
    TxSign {
        /// Parsed arguments for the signing command.
        #[command(flatten)]
        args: tx_sign::TxSignArgs,
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
            KeystoreCommand::TxSign { args } => {
                tx_sign::execute(ctx, args).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_modules_do_not_import_keystore_op_crates_or_sdk_report_helpers() {
        let modules = [
            ("delete.rs", include_str!("delete.rs")),
            ("import.rs", include_str!("import.rs")),
            ("list.rs", include_str!("list.rs")),
            ("tx_sign.rs", include_str!("tx_sign.rs")),
        ];
        let forbidden = [
            "mfm_op_keystore",
            "execute_single_op_report",
            "SingleOpReportRequest",
        ];

        for (path, source) in modules {
            for needle in forbidden {
                assert!(
                    !source.contains(needle),
                    "{path} must call mfm-app keystore services instead of importing {needle}"
                );
            }
        }
    }
}
