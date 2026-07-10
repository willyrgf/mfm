use std::fmt;

use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use clap::Subcommand;
use mfm_authored_config::EntryPointDescriptor;
use serde::Serialize;

/// Public entry-point operation discovery commands.
#[derive(Subcommand)]
pub(crate) enum OpsCommand {
    /// List the public entry-point operations registered in this binary.
    List,
}

impl OpsCommand {
    /// Dispatches the selected operation discovery command.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            Self::List => {
                handle_command_result(execute_list().await, &ctx.output_format);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct OpsOutput {
    operations: Vec<EntryPointDescriptor>,
}

impl fmt::Display for OpsOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.operations.is_empty() {
            return writeln!(f, "operations 0");
        }

        for operation in &self.operations {
            let formats = operation
                .accepted_config_formats
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            writeln!(
                f,
                "{} version={} formats={formats}",
                operation.public_name, operation.version
            )?;
        }
        Ok(())
    }
}

async fn execute_list() -> CommandResult<OpsOutput> {
    let registry = mfm_app::production_entry_point_op_registry()?;
    Ok(CommandOutput::new(OpsOutput {
        operations: registry.registered_entry_points(),
    }))
}
