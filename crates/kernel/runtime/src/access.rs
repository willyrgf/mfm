//! Affine authority for one audited application-protocol operation.

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use mfm_executor::{
    CanonicalExecutorRequest, CommittedEffectRequest, EffectExecutorOutcome, ExecutorFuture,
};
use mfm_ids::ContentRef;
use mfm_journal::v2::ValueRef;
use mfm_store::SafeFailureMetadata;

enum ReadAccess {}
enum EnsureAccess {}

/// Common private committed-request proof behind an affine access token.
struct CommittedRequest<K, T> {
    request_ref: ValueRef,
    request: T,
    _kind: PhantomData<fn(K) -> K>,
}

/// Non-cloneable authority for one registered capability invocation.
struct AuthorizedAccess<K, T> {
    request: CommittedRequest<K, T>,
}

/// Affine authority for one immutable read operation.
///
/// There is no public constructor. Runtime can create this value only by
/// consuming the store witness returned with a directly observed new
/// authorization append.
///
/// ```compile_fail
/// fn requires_clone<T: Clone>() {}
/// requires_clone::<mfm_runtime::AuthorizedReadAccess<()>>();
/// ```
#[must_use = "authorized read access must be consumed by its exact capability"]
pub struct AuthorizedReadAccess<T> {
    inner: AuthorizedAccess<ReadAccess, T>,
}

impl<T> AuthorizedReadAccess<T> {
    /// Borrows the exact state-authored typed request.
    pub const fn request(&self) -> &T {
        &self.inner.request.request
    }

    /// Returns the full immutable authority for those request bytes.
    pub const fn request_ref(&self) -> &ValueRef {
        &self.inner.request.request_ref
    }
}

/// Affine authority for one keyed executor `ensure` operation.
///
/// The committed effect request was fixed by `EffectRequested`; this token
/// grants only one audited call for that immutable request.
///
/// ```compile_fail
/// fn requires_clone<T: Clone>() {}
/// requires_clone::<mfm_runtime::AuthorizedEnsureAccess<()>>();
/// ```
#[must_use = "authorized ensure access must be consumed by its exact executor"]
pub struct AuthorizedEnsureAccess<T> {
    inner: AuthorizedAccess<EnsureAccess, CommittedEffectRequest<T>>,
}

impl<T> AuthorizedEnsureAccess<T> {
    /// Returns the exact non-authorizing committed effect request.
    pub fn committed_request(&self) -> &CommittedEffectRequest<T> {
        &self.inner.request.request
    }
}

/// Closed result of one live audited read capability invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadCapabilityOutcome<R, D> {
    /// One schema-valid typed response returned.
    Returned(R),
    /// Boundary entry was proven not to have occurred.
    DidNotEnter {
        /// Typed bounded diagnostic interpreted by the state callback.
        diagnostic: Option<D>,
        /// Reviewed generic safe-failure classification.
        metadata: SafeFailureMetadata,
    },
    /// Boundary entry or terminal outcome remains indeterminate.
    Indeterminate {
        /// Typed bounded diagnostic interpreted by the state callback.
        diagnostic: Option<D>,
        /// Reviewed generic safe-failure classification.
        metadata: SafeFailureMetadata,
    },
}

/// Future returned by one immutable read capability.
pub type ReadCapabilityFuture<'a, R, F> =
    Pin<Box<dyn Future<Output = ReadCapabilityOutcome<R, F>> + Send + 'a>>;

/// Registered implementation of one independently meaningful read operation.
///
/// The implementation receives no append, retry, redirect, failover,
/// reselection, or semantic-reduction authority.
pub trait AuditedReadCapability<Request>: Send + Sync + 'static {
    /// Schema-valid returned response.
    type Response: Send + Sync + 'static;
    /// Reviewed safe-access failure.
    type SafeDiagnostic: Send + Sync + 'static;

    /// Resolves the exact admitted routing generation for one validated typed request.
    ///
    /// Runtime calls this after the qualified request codec has verified the
    /// request contract and before it appends an authorization. Returning
    /// `None` proves that no admitted private route matches, so runtime fails
    /// closed without entering the external boundary.
    fn routing_generation_ref(&self, request: &Request) -> Option<ContentRef>;

    /// Consumes one affine authority and performs zero or one boundary entry.
    fn call<'a>(
        &'a self,
        access: AuthorizedReadAccess<Request>,
    ) -> ReadCapabilityFuture<'a, Self::Response, Self::SafeDiagnostic>;
}

/// Runtime-facing boundary for one durable keyed effect executor.
///
/// Implementations own convergence and durable delivery state but cannot
/// settle an MFM node. Runtime commits every verified pending or terminal
/// result as an observation before invoking a state settlement callback.
pub trait RecoverableEffectExecutor<Request>: Send + Sync + 'static
where
    Request: CanonicalExecutorRequest + Send + Sync + 'static,
{
    /// Consumes one affine ensure authority.
    fn ensure<'a>(
        &'a self,
        access: AuthorizedEnsureAccess<Request>,
    ) -> ExecutorFuture<'a, EffectExecutorOutcome>;
}

/// Mints read authority only by consuming a directly observed new append.
pub(crate) struct PreparedReadAccess<T> {
    request_ref: ValueRef,
    request: T,
}

impl<T> PreparedReadAccess<T> {
    pub(crate) fn new(request_ref: ValueRef, request: T) -> Self {
        Self {
            request_ref,
            request,
        }
    }

    pub(crate) fn authorize(
        self,
        witness: mfm_store::NewlyAppendedAuthorization,
    ) -> AuthorizedReadAccess<T> {
        drop(witness);
        AuthorizedReadAccess {
            inner: AuthorizedAccess {
                request: CommittedRequest {
                    request_ref: self.request_ref,
                    request: self.request,
                    _kind: PhantomData,
                },
            },
        }
    }
}

/// Fully validated typed ensure material waiting only for fresh store authority.
pub(crate) struct PreparedEnsureAccess<T> {
    request_ref: ValueRef,
    request: CommittedEffectRequest<T>,
}

impl<T> PreparedEnsureAccess<T> {
    pub(crate) fn new(request_ref: ValueRef, request: CommittedEffectRequest<T>) -> Self {
        Self {
            request_ref,
            request,
        }
    }

    pub(crate) fn authorize(
        self,
        witness: mfm_store::NewlyAppendedAuthorization,
    ) -> AuthorizedEnsureAccess<T> {
        drop(witness);
        AuthorizedEnsureAccess {
            inner: AuthorizedAccess {
                request: CommittedRequest {
                    request_ref: self.request_ref,
                    request: self.request,
                    _kind: PhantomData,
                },
            },
        }
    }
}
