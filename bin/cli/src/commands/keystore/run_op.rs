use std::sync::Arc;

use mfm_app::{
    AppError, AppServices, ArtifactBody, RunStartResponse, RunsEventsQuery, RunsStartRequest,
    SingleOpStartRequest,
};
use mfm_artifact_store_fs::FsArtifactStore;
use mfm_event_store_mem::MemEventStore;
use mfm_machine::events::{Event, KernelEvent};
use mfm_machine::stores::{ArtifactStore, EventStore};
use serde::de::DeserializeOwned;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};

fn command_error_from_app_error(err: AppError) -> CommandError {
    CommandError::new(err.code, err.message)
}

fn make_services() -> AppServices {
    let bundle = mfm_app::make_engine_bundle();

    let artifact_root = mfm_app::default_artifact_root();
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(artifact_root));
    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());

    AppServices::new(bundle, events, artifacts)
}

pub async fn execute_single_op_report<T: DeserializeOwned>(
    op_id: &str,
    op_version: &str,
    op_config: serde_json::Value,
    report_context_key: &str,
) -> CommandResult<T> {
    let services = make_services();

    let start = services
        .start_run(RunsStartRequest::Single(SingleOpStartRequest {
            op_id: op_id.to_string(),
            op_version: op_version.to_string(),
            op_config,
        }))
        .await
        .map_err(command_error_from_app_error)?;

    if start.phase != "completed" {
        return Err(error_from_failed_run(&services, &start).await);
    }

    let final_snapshot_id = start.final_snapshot_id.ok_or_else(|| {
        CommandError::new(
            "MissingFinalSnapshot",
            "run completed without a final snapshot",
        )
    })?;

    let artifact = services
        .artifact_get(&final_snapshot_id)
        .await
        .map_err(command_error_from_app_error)?;

    let snapshot = match artifact.body {
        ArtifactBody::Json { value } => value,
        ArtifactBody::Hex { .. } => {
            return Err(CommandError::new(
                "InvalidSnapshot",
                "final snapshot artifact was not JSON",
            ))
        }
    };

    let report_value = snapshot.get(report_context_key).cloned().ok_or_else(|| {
        CommandError::new(
            "MissingReport",
            "run completed without a report payload in snapshot context",
        )
    })?;

    let report: T = serde_json::from_value(report_value).map_err(|_| {
        CommandError::new(
            "InvalidReport",
            "failed to decode report payload from final snapshot",
        )
    })?;

    Ok(CommandOutput::new(report))
}

async fn error_from_failed_run(services: &AppServices, start: &RunStartResponse) -> CommandError {
    let events = services
        .run_events(
            &start.run_id,
            RunsEventsQuery {
                from_seq: 1,
                to_seq: None,
            },
        )
        .await;

    let Ok(events) = events else {
        return CommandError::new(
            "RunFailed",
            format!("run {} finished in phase {}", start.run_id, start.phase),
        );
    };

    for envelope in events.events.iter().rev() {
        let Event::Kernel(kernel) = &envelope.event else {
            continue;
        };

        if let KernelEvent::StateFailed { error, .. } = kernel {
            return CommandError::new(error.info.code.0.clone(), error.info.message.clone());
        }
    }

    CommandError::new(
        "RunFailed",
        format!("run {} finished in phase {}", start.run_id, start.phase),
    )
}
