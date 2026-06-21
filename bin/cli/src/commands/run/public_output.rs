use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::typed_run::{
    connect_run_services, parse_typed_run_id, parse_typed_schema_id, TypedRunStoresArgs,
};
use clap::Args;
use mfm_app::TypedPublicOutputResponse;

/// Arguments for `mfm run public-output`.
#[derive(Args)]
pub(crate) struct PublicOutputArgs {
    /// Typed run id (`run:<algorithm>:<digest>`)
    pub run_id: String,

    /// Public output schema id to render.
    #[arg(long)]
    pub schema_id: String,

    /// Storage configuration for certified typed run events and artifacts.
    #[command(flatten)]
    pub stores: TypedRunStoresArgs,
}

/// Executes the public-output command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &PublicOutputArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &PublicOutputArgs) -> CommandResult<TypedPublicOutputResponse> {
    let run_id = parse_typed_run_id(&args.run_id)?;
    let schema_id = parse_typed_schema_id(&args.schema_id)?;
    let services = connect_run_services(&args.stores).await?;
    let response = services.typed_public_output(&run_id, &schema_id).await?;

    Ok(CommandOutput::new(response))
}
