//! Runtime-only erased invocation wrappers stored by the program registry.

use std::any::{Any, TypeId};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use mfm_executor::{
    CanonicalExecutorRequest, CommittedEffectRequest, RequiredPlanExpansion, VerifiedEnsureResult,
    VerifiedExecutorBinding,
};
use mfm_ids::{
    ContentRef, EffectKey, NodeId, RequestDigest, RunId, StableId, StoreScopeId, TenantScopeId,
};
use mfm_journal::v1::{
    AuthorizationRef, InputManifestRef, ReadCapabilityBinding, TransitionRef, ValueRef,
};
use mfm_program::{
    ProposedValueMaterial, QualifiedAuthoredRequest, QualifiedEffectEntry, QualifiedReadEntry,
    QualifiedReadOperationContract,
};
use mfm_spec::ComponentImplementationDescriptor;

use crate::access::{
    call_ensure, call_read, mint_ensure_access, mint_read_access, ReadAccessExpectation,
};
use crate::{
    AuditedReadCapability, ReadCapabilityOutcome, RecoverableEffectExecutor, Result, RuntimeError,
};

/// Qualifies one typed audited-read implementation into the sole program entry.
///
/// The returned entry owns the complete semantic binding and a process-private
/// erased runtime invoker. Runtime accepts no second capability catalog.
pub fn qualify_read_capability<Request, Capability>(
    binding: ReadCapabilityBinding,
    operation: QualifiedReadOperationContract,
    component_descriptor: ComponentImplementationDescriptor,
    capability: Capability,
) -> Result<QualifiedReadEntry>
where
    Request: Send + Sync + 'static,
    Capability: AuditedReadCapability<Request>,
{
    QualifiedReadEntry::new::<
        Request,
        Capability::Response,
        Capability::AccessFailure,
        RuntimeReadInvoker,
    >(
        binding,
        operation,
        component_descriptor,
        RuntimeReadInvoker {
            erased: Arc::new(TypedReadCapability::<Request, Capability> {
                capability,
                _request: PhantomData,
            }),
        },
    )
    .map_err(Into::into)
}

/// Qualifies one typed recoverable effect executor into the sole program entry.
///
/// The verified executor binding remains program-owned. The process-only
/// wrapper carries only the implementation needed after exact entry selection.
pub fn qualify_effect_executor<Request, Executor>(
    operation_id: StableId,
    binding: VerifiedExecutorBinding,
    operation_contract: RequiredPlanExpansion,
    component_descriptor: ComponentImplementationDescriptor,
    executor: Executor,
) -> Result<QualifiedEffectEntry>
where
    Request: CanonicalExecutorRequest + Send + Sync + 'static,
    Executor: RecoverableEffectExecutor<Request>,
{
    QualifiedEffectEntry::new::<
        Request,
        VerifiedEnsureResult,
        mfm_executor::ExecutorError,
        RuntimeEffectInvoker,
    >(
        operation_id,
        binding,
        operation_contract,
        component_descriptor,
        RuntimeEffectInvoker {
            erased: Arc::new(TypedEffectExecutor::<Request, Executor> {
                executor,
                _request: PhantomData,
            }),
        },
    )
    .map_err(Into::into)
}

pub(crate) struct RoutedReadRequest {
    request_type: TypeId,
    request: Box<dyn Any + Send + Sync>,
    proposed: ProposedValueMaterial,
    routing_generation_ref: ContentRef,
}

impl RoutedReadRequest {
    pub(crate) const fn proposed(&self) -> &ProposedValueMaterial {
        &self.proposed
    }

    pub(crate) const fn routing_generation_ref(&self) -> &ContentRef {
        &self.routing_generation_ref
    }

    fn into_parts(
        self,
    ) -> (
        TypeId,
        Box<dyn Any + Send + Sync>,
        ProposedValueMaterial,
        ContentRef,
    ) {
        (
            self.request_type,
            self.request,
            self.proposed,
            self.routing_generation_ref,
        )
    }
}

pub(crate) enum ErasedReadOutcome {
    Returned(Box<dyn Any + Send + Sync>),
    DidNotEnter {
        failure: Box<dyn Any + Send + Sync>,
        metadata: mfm_store::SafeFailureMetadata,
    },
    Indeterminate {
        failure: Box<dyn Any + Send + Sync>,
        metadata: mfm_store::SafeFailureMetadata,
    },
}

pub(crate) struct ErasedReadObservation {
    pub(crate) authorization_ref: AuthorizationRef,
    pub(crate) outcome: ErasedReadOutcome,
}

pub(crate) struct ReadInvocationContext {
    pub(crate) binding_ref: ContentRef,
    pub(crate) operation_id: StableId,
    pub(crate) node_id: NodeId,
    pub(crate) input_manifest_ref: InputManifestRef,
    pub(crate) frozen_read_intent_ref: ValueRef,
    pub(crate) routing_generation_ref: ContentRef,
    pub(crate) safe_failure_contract_ref: ContentRef,
}

type ErasedReadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ErasedReadObservation>> + Send + 'a>>;

trait ErasedReadCapability: Send + Sync {
    fn route_request(
        &self,
        entry: &QualifiedReadEntry,
        request: QualifiedAuthoredRequest,
    ) -> Result<RoutedReadRequest>;

    fn call<'a>(
        &'a self,
        witness: mfm_store::NewlyAppendedAuthorization,
        expected_request_ref: ValueRef,
        context: ReadInvocationContext,
        request: RoutedReadRequest,
    ) -> ErasedReadFuture<'a>;
}

struct TypedReadCapability<Request, Capability> {
    capability: Capability,
    _request: PhantomData<fn(Request) -> Request>,
}

impl<Request, Capability> ErasedReadCapability for TypedReadCapability<Request, Capability>
where
    Request: Send + Sync + 'static,
    Capability: AuditedReadCapability<Request>,
{
    fn route_request(
        &self,
        entry: &QualifiedReadEntry,
        request: QualifiedAuthoredRequest,
    ) -> Result<RoutedReadRequest> {
        if entry.request_type() != TypeId::of::<Request>()
            || entry.returned_type() != TypeId::of::<Capability::Response>()
            || entry.failure_type() != TypeId::of::<Capability::AccessFailure>()
            || request.request_type() != TypeId::of::<Request>()
        {
            return Err(RuntimeError::CatalogSelection);
        }
        let proposed = request.proposed().clone();
        let request = request
            .into_value()
            .downcast::<Request>()
            .map_err(|_| RuntimeError::InvalidCallbackResult)?;
        let routing_generation_ref = self
            .capability
            .routing_generation_ref(request.as_ref())
            .ok_or(RuntimeError::CatalogSelection)?;
        if !entry
            .operation()
            .admits_routing_generation(&routing_generation_ref)
        {
            return Err(RuntimeError::CatalogSelection);
        }
        Ok(RoutedReadRequest {
            request_type: TypeId::of::<Request>(),
            request,
            proposed,
            routing_generation_ref,
        })
    }

    fn call<'a>(
        &'a self,
        witness: mfm_store::NewlyAppendedAuthorization,
        expected_request_ref: ValueRef,
        context: ReadInvocationContext,
        request: RoutedReadRequest,
    ) -> ErasedReadFuture<'a> {
        Box::pin(async move {
            let (request_type, request, _, routed_generation_ref) = request.into_parts();
            if request_type != TypeId::of::<Request>()
                || context.routing_generation_ref != routed_generation_ref
            {
                return Err(RuntimeError::AuthorityMismatch);
            }
            let request = request
                .downcast::<Request>()
                .map_err(|_| RuntimeError::InvalidCallbackResult)?;
            let access = mint_read_access(
                witness,
                ReadAccessExpectation::new(
                    &context.binding_ref,
                    &context.operation_id,
                    &context.node_id,
                    &context.input_manifest_ref,
                    &expected_request_ref,
                    &context.frozen_read_intent_ref,
                ),
                *request,
            )?;
            let observation = call_read(&self.capability, access).await;
            let outcome = match observation.outcome {
                ReadCapabilityOutcome::Returned(value) => {
                    ErasedReadOutcome::Returned(Box::new(value))
                }
                ReadCapabilityOutcome::DidNotEnter { failure, metadata } => {
                    if metadata.safe_failure_contract_ref() != &context.safe_failure_contract_ref {
                        return Err(RuntimeError::InvalidCallbackResult);
                    }
                    ErasedReadOutcome::DidNotEnter {
                        failure: Box::new(failure),
                        metadata,
                    }
                }
                ReadCapabilityOutcome::Indeterminate { failure, metadata } => {
                    if metadata.safe_failure_contract_ref() != &context.safe_failure_contract_ref {
                        return Err(RuntimeError::InvalidCallbackResult);
                    }
                    ErasedReadOutcome::Indeterminate {
                        failure: Box::new(failure),
                        metadata,
                    }
                }
            };
            Ok(ErasedReadObservation {
                authorization_ref: observation.authorization_ref,
                outcome,
            })
        })
    }
}

pub(crate) struct PreparedEffectRequest {
    pub(crate) proposed: ProposedValueMaterial,
    pub(crate) effect_key: EffectKey,
    pub(crate) request_digest: RequestDigest,
}

pub(crate) struct EffectRequestContext {
    pub(crate) binding: VerifiedExecutorBinding,
    pub(crate) operation_id: StableId,
    pub(crate) request_type: TypeId,
    pub(crate) response_type: TypeId,
    pub(crate) failure_type: TypeId,
    pub(crate) tenant_scope_id: TenantScopeId,
    pub(crate) store_scope_id: StoreScopeId,
    pub(crate) run_id: RunId,
    pub(crate) node_id: NodeId,
}

pub(crate) struct EffectInvocationContext {
    pub(crate) binding: VerifiedExecutorBinding,
    pub(crate) operation_id: StableId,
    pub(crate) request_type: TypeId,
    pub(crate) response_type: TypeId,
    pub(crate) failure_type: TypeId,
    pub(crate) tenant_scope_id: TenantScopeId,
    pub(crate) store_scope_id: StoreScopeId,
    pub(crate) run_id: RunId,
    pub(crate) node_id: NodeId,
    pub(crate) request_transition_ref: TransitionRef,
    pub(crate) effect_key: EffectKey,
    pub(crate) request_digest: RequestDigest,
}

pub(crate) struct ErasedEnsureObservation {
    pub(crate) authorization_ref: AuthorizationRef,
    pub(crate) result: VerifiedEnsureResult,
}

type ErasedEnsureFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ErasedEnsureObservation>> + Send + 'a>>;

trait ErasedEffectExecutor: Send + Sync {
    fn identify_request(
        &self,
        context: EffectRequestContext,
        request: QualifiedAuthoredRequest,
    ) -> Result<PreparedEffectRequest>;

    fn ensure<'a>(
        &'a self,
        witness: mfm_store::NewlyAppendedAuthorization,
        expected_request_ref: ValueRef,
        context: EffectInvocationContext,
        request: QualifiedAuthoredRequest,
    ) -> ErasedEnsureFuture<'a>;
}

struct TypedEffectExecutor<Request, Executor> {
    executor: Executor,
    _request: PhantomData<fn(Request) -> Request>,
}

impl<Request, Executor> ErasedEffectExecutor for TypedEffectExecutor<Request, Executor>
where
    Request: CanonicalExecutorRequest + Send + Sync + 'static,
    Executor: RecoverableEffectExecutor<Request>,
{
    fn identify_request(
        &self,
        context: EffectRequestContext,
        request: QualifiedAuthoredRequest,
    ) -> Result<PreparedEffectRequest> {
        if request.request_type() != TypeId::of::<Request>()
            || context.request_type != TypeId::of::<Request>()
            || context.response_type != TypeId::of::<VerifiedEnsureResult>()
            || context.failure_type != TypeId::of::<mfm_executor::ExecutorError>()
            || context.operation_id.is_empty()
            || context.tenant_scope_id != *context.binding.deployment().tenant_scope_id()
        {
            return Err(RuntimeError::EffectIdentityMismatch);
        }
        let proposed = request.proposed().clone();
        let request = request
            .into_value()
            .downcast::<Request>()
            .map_err(|_| RuntimeError::InvalidCallbackResult)?;
        let committed = CommittedEffectRequest::new(
            context.binding.binding_ref().clone(),
            context.tenant_scope_id,
            &context.store_scope_id,
            &context.run_id,
            &context.node_id,
            *request,
        )?;
        let canonical = committed.request().canonical_request();
        if canonical.reference()? != *proposed.content_ref()
            || canonical.as_bytes() != proposed.canonical().as_bytes()
        {
            return Err(RuntimeError::EffectIdentityMismatch);
        }
        Ok(PreparedEffectRequest {
            proposed,
            effect_key: committed.identity().effect_key().clone(),
            request_digest: committed.identity().request_digest().clone(),
        })
    }

    fn ensure<'a>(
        &'a self,
        witness: mfm_store::NewlyAppendedAuthorization,
        expected_request_ref: ValueRef,
        context: EffectInvocationContext,
        request: QualifiedAuthoredRequest,
    ) -> ErasedEnsureFuture<'a> {
        Box::pin(async move {
            if request.request_type() != TypeId::of::<Request>()
                || context.request_type != TypeId::of::<Request>()
                || context.response_type != TypeId::of::<VerifiedEnsureResult>()
                || context.failure_type != TypeId::of::<mfm_executor::ExecutorError>()
                || context.operation_id.is_empty()
                || context.tenant_scope_id != *context.binding.deployment().tenant_scope_id()
                || expected_request_ref
                    .validate_contract(context.binding.contract().semantic_request_contract())
                    .is_err()
            {
                return Err(RuntimeError::EffectIdentityMismatch);
            }
            let request = request
                .into_value()
                .downcast::<Request>()
                .map_err(|_| RuntimeError::InvalidCallbackResult)?;
            let expected = expected_request_ref.fields()?;
            let canonical = request.canonical_request();
            let content_ref = canonical.reference()?;
            let byte_length = u64::try_from(canonical.as_bytes().len())
                .map_err(|_| RuntimeError::InvalidCallbackResult)?;
            if &expected.schema_id != content_ref.schema_id()
                || &expected.content_digest != content_ref.content_digest()
                || expected.byte_length != byte_length
            {
                return Err(RuntimeError::EffectIdentityMismatch);
            }

            let committed = CommittedEffectRequest::new(
                context.binding.binding_ref().clone(),
                context.tenant_scope_id,
                &context.store_scope_id,
                &context.run_id,
                &context.node_id,
                *request,
            )?;
            if committed.identity().effect_key() != &context.effect_key
                || committed.identity().request_digest() != &context.request_digest
            {
                return Err(RuntimeError::EffectIdentityMismatch);
            }
            let binding_ref = context.binding.binding_ref().as_content_ref().clone();
            let access = mint_ensure_access(
                witness,
                &binding_ref,
                &context.operation_id,
                &context.node_id,
                &context.request_transition_ref,
                &expected_request_ref,
                committed,
            )?;
            let (authorization_ref, result) = call_ensure(&self.executor, access).await?;
            if result.identity().executor_binding_ref() != context.binding.binding_ref()
                || result.retained_closure().executor_binding_ref() != context.binding.binding_ref()
                || result.retained_closure().contract()
                    != context.binding.contract().retained_closure_contract()
            {
                return Err(RuntimeError::EffectIdentityMismatch);
            }
            Ok(ErasedEnsureObservation {
                authorization_ref,
                result,
            })
        })
    }
}

pub(crate) struct RuntimeReadInvoker {
    erased: Arc<dyn ErasedReadCapability>,
}

impl RuntimeReadInvoker {
    pub(crate) fn from_entry(entry: &QualifiedReadEntry) -> Result<&Self> {
        if entry.invoker_identity() != TypeId::of::<Self>() {
            return Err(RuntimeError::CatalogSelection);
        }
        entry
            .invoker::<Self>()
            .ok_or(RuntimeError::CatalogSelection)
    }

    pub(crate) fn route_request(
        &self,
        entry: &QualifiedReadEntry,
        request: QualifiedAuthoredRequest,
    ) -> Result<RoutedReadRequest> {
        self.erased.route_request(entry, request)
    }

    pub(crate) fn call<'a>(
        &'a self,
        witness: mfm_store::NewlyAppendedAuthorization,
        expected_request_ref: ValueRef,
        context: ReadInvocationContext,
        request: RoutedReadRequest,
    ) -> ErasedReadFuture<'a> {
        self.erased
            .call(witness, expected_request_ref, context, request)
    }
}

pub(crate) struct RuntimeEffectInvoker {
    erased: Arc<dyn ErasedEffectExecutor>,
}

impl RuntimeEffectInvoker {
    pub(crate) fn from_entry(entry: &QualifiedEffectEntry) -> Result<&Self> {
        if entry.invoker_identity() != TypeId::of::<Self>() {
            return Err(RuntimeError::CatalogSelection);
        }
        entry
            .invoker::<Self>()
            .ok_or(RuntimeError::CatalogSelection)
    }

    pub(crate) fn identify_request(
        &self,
        context: EffectRequestContext,
        request: QualifiedAuthoredRequest,
    ) -> Result<PreparedEffectRequest> {
        self.erased.identify_request(context, request)
    }

    pub(crate) fn ensure<'a>(
        &'a self,
        witness: mfm_store::NewlyAppendedAuthorization,
        expected_request_ref: ValueRef,
        context: EffectInvocationContext,
        request: QualifiedAuthoredRequest,
    ) -> ErasedEnsureFuture<'a> {
        self.erased
            .ensure(witness, expected_request_ref, context, request)
    }
}
