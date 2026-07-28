use clap::Subcommand;

use crate::commands::CommandContext;
use crate::presentation::output::handle_public_result;
use crate::support::application::{connect_application, ApplicationConnectionArgs};

/// Public entry-point discovery commands.
#[derive(Subcommand)]
pub(crate) enum OpsCommand {
    /// List every complete compiled entry-point contract.
    List {
        /// Non-semantic process connection options.
        #[command(flatten)]
        connection: ApplicationConnectionArgs,
    },
}

impl OpsCommand {
    /// Dispatches the selected discovery command.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            Self::List { connection } => {
                let result = connect_application(connection)
                    .await
                    .map(|application| application.entry_points().to_vec());
                handle_public_result(result, &ctx.output_format, |entries| {
                    let mut text = entries
                        .iter()
                        .map(|entry| entry.entry_point_id().as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    Ok(text)
                })
            }
        }
    }
}
