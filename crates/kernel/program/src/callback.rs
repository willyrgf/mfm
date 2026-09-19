//! Typed invocation primitives shared by Program callbacks and the Runtime kernel.
//!
//! This boundary retains the failure phase and immutable diagnostic only. It has no
//! run, Store, Journal, acknowledgement or classification authority.

mod adapter;
mod native;
pub use adapter::*;
mod state;
pub use state::*;

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use mfm_capabilities::AdapterError;
use mfm_ids::{ContentRef, ExecutionPosition};
use mfm_values::{InvocationDiagnostic, MfmValue, Object};
use serde::Serialize;

use mfm_capabilities::CallbackFailure;

fn task_failure(operation: &'static str, error: tokio::task::JoinError) -> InvocationDiagnostic {
    InvocationDiagnostic::from_fields(
        "task_failure",
        operation,
        if error.is_panic() {
            "panicked"
        } else {
            "cancelled"
        },
        None,
    )
}

/// Decodes a retained Object on an immediately awaited pure blocking job.
async fn decode<T: MfmValue>(object: Object) -> Result<T, CallbackFailure> {
    tokio::task::spawn_blocking(move || mfm_capabilities::codec::decode(|| object.decode::<T>()))
        .await
        .map_err(|source| CallbackFailure::Decode(task_failure("decode", source)))?
}

/// Encodes one proposed value exactly once on an immediately awaited pure blocking job.
async fn encode<T: MfmValue>(value: T) -> Result<Object, CallbackFailure> {
    tokio::task::spawn_blocking(move || {
        mfm_capabilities::codec::encode(|| {
            Object::from_value(&value).map_err(|source| source.into_diagnostic("encode"))
        })
    })
    .await
    .map_err(|source| CallbackFailure::Encode(task_failure("encode", source)))?
}

/// Encodes a declared original once, retaining its known slot when admission fails.
async fn encode_failure<E: MfmValue>(
    error: E,
    position: ExecutionPosition,
    failure_contract: &ContentRef,
) -> Result<Object, CallbackFailure> {
    #[derive(Serialize)]
    struct Fields<'a, T: ?Sized> {
        encoding_target: &'static str,
        position: ExecutionPosition,
        failure_contract: &'a ContentRef,
        original_detail: &'static str,
        original_identity: &'static str,
        encoding: &'a T,
    }
    let result = tokio::task::spawn_blocking(move || Object::from_value(&error)).await;
    let diagnostic = match result {
        Ok(Ok(object)) => return Ok(object),
        Ok(Err(error)) => {
            let size = match &error {
                mfm_values::ValueError::SizeLimit(size) => {
                    Some(mfm_values::SizeViolation::measured(
                        mfm_values::SizeResource::CanonicalObject,
                        *size,
                    ))
                }
                mfm_values::ValueError::Canonical(source) => {
                    source
                        .serialization_bound()
                        .map(|(limit, observed_at_least)| {
                            mfm_values::SizeViolation::SerializationBound {
                                resource: mfm_values::SizeResource::CanonicalObject,
                                limit: limit as u64,
                                observed_at_least: observed_at_least as u64,
                            }
                        })
                }
                _ => None,
            };
            InvocationDiagnostic::from_fields(
                "value_error",
                "encode_failure",
                &Fields {
                    encoding_target: "declared_failure",
                    position,
                    failure_contract,
                    original_detail: "unavailable",
                    original_identity: "unavailable",
                    encoding: &error,
                },
                size,
            )
        }
        Err(error) => InvocationDiagnostic::from_fields(
            "task_failure",
            "encode_failure",
            &Fields {
                encoding_target: "declared_failure",
                position,
                failure_contract,
                original_detail: "unavailable",
                original_identity: "unavailable",
                encoding: if error.is_panic() {
                    "panicked"
                } else {
                    "cancelled"
                },
            },
            None,
        ),
    };
    Err(CallbackFailure::Encode(diagnostic))
}

type AdapterFuture<'a, T, E> =
    Pin<Box<dyn Future<Output = Result<T, AdapterError<E>>> + Send + 'a>>;

struct CatchAdapterPanic<'a, T, E> {
    inner: AdapterFuture<'a, T, E>,
}
impl<T, E> Future for CatchAdapterPanic<'_, T, E> {
    type Output = Result<T, AdapterError<E>>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.inner.as_mut().poll(context)
        }))
        .unwrap_or_else(|_| {
            Poll::Ready(Err(AdapterError::Invariant(
                InvocationDiagnostic::from_fields("task_failure", "poll", "panicked", None),
            )))
        })
    }
}

/// Invokes an async adapter without moving IO to a blocking worker.
///
/// Constructor and poll panics become Execute failures without their payloads.
/// A returned operational original remains typed and unclassified for its one encoding.
/// Dropping this future drops the in-flight adapter; it makes no acknowledgement claim.
async fn invoke_adapter<'a, T, E>(
    create: impl FnOnce() -> AdapterFuture<'a, T, E>,
) -> Result<Result<T, E>, CallbackFailure> {
    let future = std::panic::catch_unwind(std::panic::AssertUnwindSafe(create)).map_err(|_| {
        CallbackFailure::Execute(InvocationDiagnostic::from_fields(
            "task_failure",
            "invoke_adapter",
            "panicked",
            None,
        ))
    })?;
    match (CatchAdapterPanic { inner: future }).await {
        Ok(evidence) => Ok(Ok(evidence)),
        Err(AdapterError::Operational(original)) => Ok(Err(original)),
        Err(AdapterError::Invariant(cause)) => Err(CallbackFailure::Execute(cause)),
    }
}
