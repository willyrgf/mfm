use std::fmt;

use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use clap::Subcommand;
use mfm_app::EntryPointSummary;
use serde::Serialize;

/// Public entry-point discovery commands.
#[derive(Subcommand)]
pub(crate) enum OpsCommand {
    /// List the exact entry-point ids in this binary.
    List,
}

impl OpsCommand {
    /// Dispatches the selected discovery command.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            Self::List => handle_command_result(execute_list().await, &ctx.output_format),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct OpsOutput {
    entry_points: Vec<EntryPointSummary>,
}

impl fmt::Display for OpsOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for entry_point in &self.entry_points {
            writeln!(f, "{}", entry_point.entry_point_id)?;
        }
        Ok(())
    }
}

async fn execute_list() -> CommandResult<OpsOutput> {
    Ok(CommandOutput::new(OpsOutput {
        entry_points: mfm_app::entry_point_summaries(),
    }))
}
