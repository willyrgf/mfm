//! Affine authority for one audited application-protocol operation.

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use mfm_executor::{
    CanonicalExecutorRequest, CommittedEffectRequest, ExecutorFuture, VerifiedEnsureResult,
};
use mfm_ids::{ContentRef, NodeId, StableId};
use mfm_journal::v1::{
    AuthorizationRef, AuthorizationScopeFields, InputManifestRef, ObservationRef, TransitionRef,
    ValueRef,
};
use mfm_store::SafeFailureMetadata;

use crate::{Result, RuntimeError};

enum ReadAccess {}
enum EnsureAccess {}

/// Common private committed-request proof behind an affine access token.
struct CommittedRequest<K, T> {
    authorization_ref: AuthorizationRef,
    request_ref: ValueRef,
    request: T,
    _kind: PhantomData<fn(K) -> K>,
}

/// Private proof that one exact observation is already committed.
pub(crate) struct CommittedObservation<T> {
    observation_ref: ObservationRef,
    outcome: T,
}

impl<T> CommittedObservation<T> {
    pub(crate) fn new(observation_ref: ObservationRef, outcome: T) -> Self {
        Self {
            observation_ref,
            outcome,
        }
    }

    pub(crate) fn observation_ref(&self) -> &ObservationRef {
        &self.observation_ref
    }

    pub(crate) fn outcome(&self) -> &T {
        &self.outcome
    }
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

    fn authorization_ref(&self) -> AuthorizationRef {
        self.inner.request.authorization_ref.clone()
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

    fn authorization_ref(&self) -> AuthorizationRef {
        self.inner.request.authorization_ref.clone()
    }
}

/// Closed result of one live audited read capability invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadCapabilityOutcome<R, F> {
    /// One schema-valid typed response returned.
    Returned(R),
    /// Boundary entry was proven not to have occurred.
    DidNotEnter {
        /// Typed bounded diagnostic interpreted by the state callback.
        failure: F,
        /// Reviewed generic safe-failure classification.
        metadata: SafeFailureMetadata,
    },
    /// Boundary entry or terminal outcome remains indeterminate.
    Indeterminate {
        /// Typed bounded diagnostic interpreted by the state callback.
        failure: F,
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
    type AccessFailure: Send + Sync + 'static;

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
    ) -> ReadCapabilityFuture<'a, Self::Response, Self::AccessFailure>;
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
    ) -> ExecutorFuture<'a, mfm_executor::Result<VerifiedEnsureResult>>;
}

/// Sealed wrapper that can be converted into an observation append.
pub(crate) struct UncommittedAccessObservation<R, F> {
    pub(crate) authorization_ref: AuthorizationRef,
    pub(crate) outcome: ReadCapabilityOutcome<R, F>,
}

/// Calls one read capability while retaining only non-authorizing metadata.
pub(crate) async fn call_read<Request, Capability>(
    capability: &Capability,
    access: AuthorizedReadAccess<Request>,
) -> UncommittedAccessObservation<Capability::Response, Capability::AccessFailure>
where
    Request: Send + Sync + 'static,
    Capability: AuditedReadCapability<Request>,
{
    let authorization_ref = access.authorization_ref();
    let outcome = capability.call(access).await;
    UncommittedAccessObservation {
        authorization_ref,
        outcome,
    }
}

/// Calls one effect executor while retaining only non-authorizing metadata.
pub(crate) async fn call_ensure<Request, Executor>(
    executor: &Executor,
    access: AuthorizedEnsureAccess<Request>,
) -> mfm_executor::Result<(AuthorizationRef, VerifiedEnsureResult)>
where
    Request: CanonicalExecutorRequest + Send + Sync + 'static,
    Executor: RecoverableEffectExecutor<Request>,
{
    let authorization_ref = access.authorization_ref();
    let result = executor.ensure(access).await?;
    Ok((authorization_ref, result))
}

/// Mints read authority only by consuming a directly observed new append.
pub(crate) struct ReadAccessExpectation<'a> {
    binding_ref: &'a ContentRef,
    operation_id: &'a StableId,
    node_id: &'a NodeId,
    input_manifest_ref: &'a InputManifestRef,
    request_ref: &'a ValueRef,
    frozen_read_intent_ref: &'a ValueRef,
}

impl<'a> ReadAccessExpectation<'a> {
    pub(crate) const fn new(
        binding_ref: &'a ContentRef,
        operation_id: &'a StableId,
        node_id: &'a NodeId,
        input_manifest_ref: &'a InputManifestRef,
        request_ref: &'a ValueRef,
        frozen_read_intent_ref: &'a ValueRef,
    ) -> Self {
        Self {
            binding_ref,
            operation_id,
            node_id,
            input_manifest_ref,
            request_ref,
            frozen_read_intent_ref,
        }
    }
}

pub(crate) fn mint_read_access<T>(
    witness: mfm_store::NewlyAppendedAuthorization,
    expected: ReadAccessExpectation<'_>,
    request: T,
) -> Result<AuthorizedReadAccess<T>> {
    let authorization = witness.authorization().fields()?;
    if authorization.capability_binding_ref.fields()? != *expected.binding_ref
        || authorization.capability_operation_id != *expected.operation_id
        || authorization.request_ref != *expected.request_ref
        || authorization.semantic_anchor.fields()?.node_id != *expected.node_id
        || authorization.frozen_read_intent_ref.as_ref() != Some(expected.frozen_read_intent_ref)
        || !matches!(
            authorization.scope.fields()?,
            AuthorizationScopeFields::Read { input_manifest_ref }
                if input_manifest_ref == *expected.input_manifest_ref
        )
    {
        return Err(RuntimeError::AuthorityMismatch);
    }
    Ok(AuthorizedReadAccess {
        inner: AuthorizedAccess {
            request: CommittedRequest {
                authorization_ref: witness.authorization_ref().clone(),
                request_ref: expected.request_ref.clone(),
                request,
                _kind: PhantomData,
            },
        },
    })
}

/// Mints ensure authority only by consuming a directly observed new append.
pub(crate) fn mint_ensure_access<T>(
    witness: mfm_store::NewlyAppendedAuthorization,
    expected_binding_ref: &ContentRef,
    expected_operation_id: &StableId,
    expected_node_id: &NodeId,
    expected_request_transition_ref: &TransitionRef,
    expected_request_ref: &ValueRef,
    request: CommittedEffectRequest<T>,
) -> Result<AuthorizedEnsureAccess<T>> {
    let authorization = witness.authorization().fields()?;
    if authorization.capability_binding_ref.fields()? != *expected_binding_ref
        || authorization.capability_operation_id != *expected_operation_id
        || authorization.request_ref != *expected_request_ref
        || authorization.semantic_anchor.fields()?.node_id != *expected_node_id
        || authorization.frozen_read_intent_ref.is_some()
        || !matches!(
            authorization.scope.fields()?,
            AuthorizationScopeFields::EnsureEffect {
                effect_request_transition_ref,
            } if effect_request_transition_ref == *expected_request_transition_ref
        )
    {
        return Err(RuntimeError::AuthorityMismatch);
    }
    Ok(AuthorizedEnsureAccess {
        inner: AuthorizedAccess {
            request: CommittedRequest {
                authorization_ref: witness.authorization_ref().clone(),
                request_ref: expected_request_ref.clone(),
                request,
                _kind: PhantomData,
            },
        },
    })
}
