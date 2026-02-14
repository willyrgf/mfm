//! Attempt-level execution pipeline (typed middlewares).
//!
//! Kept in a submodule to keep `runtime.rs` smaller and to centralize cross-cutting behavior
//! (skip tags, redaction, failpoints) in a single, ordered pipeline.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, info, warn};

use crate::config::{IoMode, RunConfig};
use crate::context::DynContext;
use crate::context_runtime::{read_json_context, write_full_snapshot_value, StagedContext};
use crate::engine::Stores;
use crate::errors::{ErrorCategory, IoError, RunError};
use crate::events::{DomainEvent, Event, FactRecorded, KernelEvent, DOMAIN_EVENT_FACT_RECORDED};
use crate::ids::{ArtifactId, RunId, StateId};
use crate::io::{IoCall, IoProvider, IoResult};
use crate::live_io::{FactIndex, FactRecorder, LiveIo, LiveIoEnv, LiveIoTransportFactory};
use crate::recorder::EventRecorder;
use crate::replay_io::ReplayIo;

use super::{EngineFailpoints, SharedEventWriter};

fn should_skip_state(run_config: &RunConfig, state_meta: &crate::meta::StateMeta) -> bool {
    if run_config.skip_tags.is_empty() {
        return false;
    }

    state_meta
        .tags
        .iter()
        .any(|t| run_config.skip_tags.contains(t))
}

pub(super) struct AttemptCtx<'a> {
    stores: &'a Stores,
    run_config: &'a RunConfig,
    run_id: RunId,
    state_id: StateId,
    attempt: u32,
    base_snapshot_id: ArtifactId,
    facts: FactIndex,
    writer: SharedEventWriter,
    live_factory: Arc<dyn LiveIoTransportFactory>,
    failpoints: Option<EngineFailpoints>,
    state: crate::state::DynState,
    state_meta: crate::meta::StateMeta,
}

impl<'a> AttemptCtx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        stores: &'a Stores,
        run_config: &'a RunConfig,
        run_id: RunId,
        state_id: StateId,
        attempt: u32,
        base_snapshot_id: ArtifactId,
        facts: FactIndex,
        writer: SharedEventWriter,
        live_factory: Arc<dyn LiveIoTransportFactory>,
        failpoints: Option<EngineFailpoints>,
        state: crate::state::DynState,
        state_meta: crate::meta::StateMeta,
    ) -> Self {
        Self {
            stores,
            run_config,
            run_id,
            state_id,
            attempt,
            base_snapshot_id,
            facts,
            writer,
            live_factory,
            failpoints,
            state,
            state_meta,
        }
    }
}

const CODE_FACT_BINDING_APPEND_FAILED: &str = "fact_binding_append_failed";
const CODE_FACT_BINDING_PAYLOAD_INVALID: &str = "fact_binding_payload_invalid";

#[derive(Clone)]
struct RuntimeFactRecorder {
    writer: SharedEventWriter,
}

#[async_trait]
impl FactRecorder for RuntimeFactRecorder {
    async fn record_fact_binding(
        &self,
        key: crate::ids::FactKey,
        payload_id: ArtifactId,
    ) -> Result<(), IoError> {
        if crate::secrets::string_contains_secrets(&key.0) {
            return Err(IoError::Other(super::info(
                "secrets_detected",
                ErrorCategory::Unknown,
                "fact key contained secrets (Milestone 1 forbids persisting secrets)",
            )));
        }

        let payload = serde_json::to_value(FactRecorded {
            key,
            payload_id,
            meta: serde_json::json!({}),
        })
        .map_err(|_| {
            IoError::Other(super::info(
                CODE_FACT_BINDING_PAYLOAD_INVALID,
                ErrorCategory::ParsingInput,
                "failed to serialize FactRecorded payload",
            ))
        })?;

        if crate::secrets::json_contains_secrets(&payload) {
            return Err(IoError::Other(super::info(
                "secrets_detected",
                ErrorCategory::Unknown,
                "fact binding payload contained secrets (Milestone 1 forbids persisting secrets)",
            )));
        }

        let event = DomainEvent {
            name: DOMAIN_EVENT_FACT_RECORDED.to_string(),
            payload,
            payload_ref: None,
        };

        self.writer
            .lock()
            .await
            .append(vec![Event::Domain(event)])
            .await
            .map_err(|_| {
                IoError::Other(super::info(
                    CODE_FACT_BINDING_APPEND_FAILED,
                    ErrorCategory::Storage,
                    "failed to append fact binding event",
                ))
            })?;

        Ok(())
    }
}

struct AppendEventRecorder {
    writer: SharedEventWriter,
}

#[async_trait]
impl EventRecorder for AppendEventRecorder {
    async fn emit(&mut self, event: DomainEvent) -> Result<(), RunError> {
        if crate::secrets::string_contains_secrets(&event.name)
            || crate::secrets::json_contains_secrets(&event.payload)
        {
            return Err(RunError::Other(super::info(
                "secrets_detected",
                ErrorCategory::Unknown,
                "domain event contained secrets (Milestone 1 forbids persisting secrets)",
            )));
        }

        self.writer
            .lock()
            .await
            .append(vec![Event::Domain(event)])
            .await
            .map_err(RunError::Storage)?;
        Ok(())
    }

    async fn emit_many(&mut self, events: Vec<DomainEvent>) -> Result<(), RunError> {
        for e in &events {
            if crate::secrets::string_contains_secrets(&e.name)
                || crate::secrets::json_contains_secrets(&e.payload)
            {
                return Err(RunError::Other(super::info(
                    "secrets_detected",
                    ErrorCategory::Unknown,
                    "domain event contained secrets (Milestone 1 forbids persisting secrets)",
                )));
            }
        }

        self.writer
            .lock()
            .await
            .append(events.into_iter().map(Event::Domain).collect())
            .await
            .map_err(RunError::Storage)?;
        Ok(())
    }
}

enum AttemptIo {
    Live(LiveIo),
    Replay(ReplayIo),
}

#[async_trait]
impl IoProvider for AttemptIo {
    async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
        match self {
            AttemptIo::Live(io) => io.call(call).await,
            AttemptIo::Replay(io) => io.call(call).await,
        }
    }

    async fn get_recorded_fact(
        &mut self,
        key: &crate::ids::FactKey,
    ) -> Result<Option<ArtifactId>, IoError> {
        match self {
            AttemptIo::Live(io) => io.get_recorded_fact(key).await,
            AttemptIo::Replay(io) => io.get_recorded_fact(key).await,
        }
    }

    async fn now_millis(&mut self) -> Result<u64, IoError> {
        match self {
            AttemptIo::Live(io) => io.now_millis().await,
            AttemptIo::Replay(io) => io.now_millis().await,
        }
    }

    async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
        match self {
            AttemptIo::Live(io) => io.random_bytes(n).await,
            AttemptIo::Replay(io) => io.random_bytes(n).await,
        }
    }
}

enum HandlerResult {
    Ok(StagedContext),
    Err(crate::errors::StateError),
}

enum AfterHandlerResult {
    StopAfterHandler,
    Done(HandlerResult),
}

#[derive(Clone)]
struct SanitizedStateError(crate::errors::StateError);

impl SanitizedStateError {
    fn new(mut err: crate::errors::StateError, state_id: &StateId) -> Self {
        if err.state_id.is_none() {
            err.state_id = Some(state_id.clone());
        }

        // Milestone 1: never persist secrets in error details.
        crate::secrets::redact_error_info(&mut err.info);

        Self(err)
    }

    fn retryable(&self) -> bool {
        self.0.info.retryable
    }
}

enum RedactedResult {
    StopAfterHandler,
    Ok(StagedContext),
    Err(SanitizedStateError),
}

enum AttemptOutcome {
    Skipped,
    StopAfterHandler,
    Ok(StagedContext),
    Err(SanitizedStateError),
}

#[async_trait]
trait AttemptStep {
    type Output;

    async fn run(&self, ctx: &mut AttemptCtx<'_>) -> Result<Self::Output, RunError>;
}

struct CallHandler;

#[async_trait]
impl AttemptStep for CallHandler {
    type Output = HandlerResult;

    async fn run(&self, ctx: &mut AttemptCtx<'_>) -> Result<Self::Output, RunError> {
        debug!(
            run_id = %ctx.run_id.0,
            state_id = %ctx.state_id.0,
            attempt = ctx.attempt,
            io_mode = ?ctx.run_config.io_mode,
            "executing state handler"
        );
        let base_ctx =
            read_json_context(ctx.stores.artifacts.as_ref(), &ctx.base_snapshot_id).await?;
        let mut staged = StagedContext::new(base_ctx);

        let mut io = match ctx.run_config.io_mode {
            IoMode::Live => {
                let fact_recorder: Arc<dyn FactRecorder> = Arc::new(RuntimeFactRecorder {
                    writer: Arc::clone(&ctx.writer),
                });
                AttemptIo::Live(LiveIo::new(
                    ctx.run_id,
                    ctx.state_id.clone(),
                    ctx.attempt,
                    Arc::clone(&ctx.stores.artifacts),
                    ctx.facts.clone(),
                    fact_recorder,
                    ctx.live_factory.make(LiveIoEnv {
                        stores: ctx.stores.clone(),
                        run_id: ctx.run_id,
                        state_id: ctx.state_id.clone(),
                        attempt: ctx.attempt,
                    }),
                ))
            }
            IoMode::Replay => AttemptIo::Replay(ReplayIo::new(
                ctx.run_id,
                ctx.state_id.clone(),
                ctx.attempt,
                Arc::clone(&ctx.stores.artifacts),
                ctx.facts.clone(),
                ctx.run_config.replay_missing_fact_retryable,
            )),
        };

        let mut append_rec = AppendEventRecorder {
            writer: Arc::clone(&ctx.writer),
        };
        let mut rec = crate::event_profile::FilteringEventRecorder::new(
            ctx.run_config.event_profile.clone(),
            &mut append_rec,
        );

        match ctx.state.handle(&mut staged, &mut io, &mut rec).await {
            Ok(_) => Ok(HandlerResult::Ok(staged)),
            Err(err) => Ok(HandlerResult::Err(err)),
        }
    }
}

struct WithFailpoints<S> {
    inner: S,
}

#[async_trait]
impl<S> AttemptStep for WithFailpoints<S>
where
    S: AttemptStep<Output = HandlerResult> + Send + Sync,
{
    type Output = AfterHandlerResult;

    async fn run(&self, ctx: &mut AttemptCtx<'_>) -> Result<Self::Output, RunError> {
        let out = self.inner.run(ctx).await?;
        if let Some(fp) = &ctx.failpoints {
            if fp.should_stop_after_handler() {
                return Ok(AfterHandlerResult::StopAfterHandler);
            }
        }
        Ok(AfterHandlerResult::Done(out))
    }
}

struct RedactErrors<S> {
    inner: S,
}

#[async_trait]
impl<S> AttemptStep for RedactErrors<S>
where
    S: AttemptStep<Output = AfterHandlerResult> + Send + Sync,
{
    type Output = RedactedResult;

    async fn run(&self, ctx: &mut AttemptCtx<'_>) -> Result<Self::Output, RunError> {
        match self.inner.run(ctx).await? {
            AfterHandlerResult::StopAfterHandler => Ok(RedactedResult::StopAfterHandler),
            AfterHandlerResult::Done(HandlerResult::Ok(staged)) => Ok(RedactedResult::Ok(staged)),
            AfterHandlerResult::Done(HandlerResult::Err(err)) => Ok(RedactedResult::Err(
                SanitizedStateError::new(err, &ctx.state_id),
            )),
        }
    }
}

struct SkipTags<S> {
    inner: S,
}

#[async_trait]
impl<S> AttemptStep for SkipTags<S>
where
    S: AttemptStep<Output = RedactedResult> + Send + Sync,
{
    type Output = AttemptOutcome;

    async fn run(&self, ctx: &mut AttemptCtx<'_>) -> Result<Self::Output, RunError> {
        if should_skip_state(ctx.run_config, &ctx.state_meta) {
            return Ok(AttemptOutcome::Skipped);
        }

        match self.inner.run(ctx).await? {
            RedactedResult::StopAfterHandler => Ok(AttemptOutcome::StopAfterHandler),
            RedactedResult::Ok(staged) => Ok(AttemptOutcome::Ok(staged)),
            RedactedResult::Err(err) => Ok(AttemptOutcome::Err(err)),
        }
    }
}

pub(super) enum AttemptExec {
    Completed { snapshot_id: ArtifactId },
    Failed { retryable: bool },
    StopAfterHandler,
}

pub(super) async fn execute_attempt(ctx: &mut AttemptCtx<'_>) -> Result<AttemptExec, RunError> {
    info!(
        run_id = %ctx.run_id.0,
        state_id = %ctx.state_id.0,
        attempt = ctx.attempt,
        base_snapshot_id = %ctx.base_snapshot_id.0,
        "state attempt started"
    );
    super::append_kernel(
        &ctx.writer,
        KernelEvent::StateEntered {
            state_id: ctx.state_id.clone(),
            attempt: ctx.attempt,
            base_snapshot_id: ctx.base_snapshot_id.clone(),
        },
    )
    .await?;

    let logic = SkipTags {
        inner: RedactErrors {
            inner: WithFailpoints { inner: CallHandler },
        },
    };

    match logic.run(ctx).await? {
        AttemptOutcome::StopAfterHandler => {
            warn!(
                run_id = %ctx.run_id.0,
                state_id = %ctx.state_id.0,
                attempt = ctx.attempt,
                "state attempt interrupted by failpoint"
            );
            Ok(AttemptExec::StopAfterHandler)
        }
        AttemptOutcome::Skipped => {
            info!(
                run_id = %ctx.run_id.0,
                state_id = %ctx.state_id.0,
                attempt = ctx.attempt,
                "state skipped due to run config tags"
            );
            super::append_kernel(
                &ctx.writer,
                KernelEvent::StateCompleted {
                    state_id: ctx.state_id.clone(),
                    context_snapshot_id: ctx.base_snapshot_id.clone(),
                },
            )
            .await?;
            Ok(AttemptExec::Completed {
                snapshot_id: ctx.base_snapshot_id.clone(),
            })
        }
        AttemptOutcome::Ok(staged) => {
            let snapshot = staged.dump().map_err(RunError::Context)?;
            let snapshot_id =
                write_full_snapshot_value(ctx.stores.artifacts.as_ref(), snapshot).await?;
            info!(
                run_id = %ctx.run_id.0,
                state_id = %ctx.state_id.0,
                attempt = ctx.attempt,
                snapshot_id = %snapshot_id.0,
                "state attempt completed"
            );
            super::append_kernel(
                &ctx.writer,
                KernelEvent::StateCompleted {
                    state_id: ctx.state_id.clone(),
                    context_snapshot_id: snapshot_id.clone(),
                },
            )
            .await?;
            Ok(AttemptExec::Completed { snapshot_id })
        }
        AttemptOutcome::Err(err) => {
            warn!(
                run_id = %ctx.run_id.0,
                state_id = %ctx.state_id.0,
                attempt = ctx.attempt,
                error_code = %err.0.info.code.0,
                retryable = err.retryable(),
                "state attempt failed"
            );
            super::append_kernel(
                &ctx.writer,
                KernelEvent::StateFailed {
                    state_id: ctx.state_id.clone(),
                    error: err.0.clone(),
                    failure_snapshot_id: None,
                },
            )
            .await?;
            Ok(AttemptExec::Failed {
                retryable: err.retryable(),
            })
        }
    }
}
