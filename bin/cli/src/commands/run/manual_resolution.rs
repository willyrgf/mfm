use std::path::PathBuf;

use crate::commands::result::{CommandOutput, CommandResult, PublicError};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_store::{connect_run_services, parse_run_id, RunStoresArgs};
use clap::{Args, ValueEnum};
use mfm_app::{ManualResolutionDecision, ManualResolutionRecordRequest, RunResponse};
use mfm_canonical::PlainCanonicalJsonBytes;

/// Arguments for `mfm run manual-resolution`.
#[derive(Args)]
pub(crate) struct ManualResolutionArgs {
    /// Typed run id (`run:<algorithm>:<digest>`)
    pub run_id: String,

    /// Operator-selected manual resolution outcome.
    #[arg(long, value_enum)]
    pub outcome: ManualResolutionOutcomeArg,

    /// Evidence artifact bytes covered by the signed authorization proof.
    #[arg(long, value_name = "PATH")]
    pub evidence: PathBuf,

    /// Canonical or canonicalizable manual authorization proof JSON.
    #[arg(long = "authorization-proof", value_name = "PATH")]
    pub authorization_proof: PathBuf,

    /// Media type to record for the evidence artifact.
    #[arg(long, default_value = "application/json")]
    pub evidence_media_type: String,

    /// Optional redaction-safe operator note to attach to the resolution event.
    #[arg(long)]
    pub note: Option<String>,

    /// Storage configuration for certified typed run events and artifacts.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Manual-resolution outcomes accepted by the CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ManualResolutionOutcomeArg {
    /// Confirm that unresolved obligations were remediated externally.
    ConfirmRemediated,
    /// Close the run without a compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
}

impl ManualResolutionOutcomeArg {
    fn into_app(self) -> ManualResolutionDecision {
        match self {
            Self::ConfirmRemediated => ManualResolutionDecision::ConfirmRemediated,
            Self::FailWithoutAcdcClaim => ManualResolutionDecision::FailWithoutAcdcClaim,
        }
    }
}

/// Executes the manual-resolution command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &ManualResolutionArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ManualResolutionArgs) -> CommandResult<RunResponse> {
    let run_id = parse_run_id(&args.run_id)?;
    let evidence_bytes = tokio::fs::read(&args.evidence).await.map_err(|_| {
        PublicError::internal(
            "ManualResolutionEvidenceReadFailed",
            "Failed to read manual resolution evidence file",
        )
    })?;
    let proof_bytes = read_canonical_proof_json(&args.authorization_proof).await?;
    let services = connect_run_services(&args.stores, None).await?;
    let response = services
        .record_manual_resolution(ManualResolutionRecordRequest {
            run_id,
            outcome: args.outcome.into_app(),
            evidence_bytes,
            evidence_media_type: args.evidence_media_type.clone(),
            authorization_proof_bytes: proof_bytes,
            note: args.note.clone(),
        })
        .await?;
    Ok(CommandOutput::new(response))
}

async fn read_canonical_proof_json(path: &PathBuf) -> Result<Vec<u8>, PublicError> {
    let raw = tokio::fs::read_to_string(path).await.map_err(|_| {
        PublicError::internal(
            "ManualResolutionProofReadFailed",
            "Failed to read manual authorization proof file",
        )
    })?;
    PlainCanonicalJsonBytes::from_json_str(&raw)
        .map(|canonical| canonical.to_vec())
        .map_err(|_| {
            PublicError::internal(
                "ManualResolutionProofInvalid",
                "Manual authorization proof file is not canonical JSON",
            )
        })
}
