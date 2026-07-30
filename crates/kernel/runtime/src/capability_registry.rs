//! Runtime-only erased invocation wrappers stored by the program registry.

use std::any::{Any, TypeId};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use mfm_executor::{
    CanonicalExecutorRequest, CommittedEffectRequest, EffectExecutorOutcome,
    EffectExecutorOutcomeView, RequiredPlanExpansion, VerifiedEnsureResult,
    VerifiedExecutorBinding,
};
use mfm_ids::{
    ContentRef, EffectKey, NodeId, RequestDigest, RunId, StableId, StoreScopeId, TenantScopeId,
};
use mfm_journal::v2::{
    AuthorizationRef, NonDomainDisposition, NonDomainEntryStatus, NonDomainFailure,
    NonDomainFailureCode, ReadCapabilityBinding, ValueRef,
};
use mfm_program::{
    ProposedValueMaterial, QualifiedAuthoredRequest, QualifiedEffectEntry, QualifiedReadEntry,
    QualifiedReadOperationContract,
};
use mfm_spec::ComponentImplementationDescriptor;

use crate::access::{PreparedEnsureAccess, PreparedReadAccess};
use crate::runtime_error::non_domain_failure;
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
        Capability::SafeDiagnostic,
        RuntimeReadInvoker,
    >(
        binding,
        operation,
        component_descriptor,
        RuntimeReadInvoker {
            erased: Arc::new(TypedReadCapability::<Request, Capability> {
                capability: Arc::new(capability),
                _request: PhantomData,
            }),
            adapter_may_have_entered: non_domain_failure(
                NonDomainEntryStatus::MayHaveEntered,
                NonDomainDisposition::IntegrityBlocked,
                NonDomainFailureCode::AdapterContractViolation,
            )?,
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
                executor: Arc::new(executor),
                _request: PhantomData,
            }),
            adapter_may_have_entered: EffectExecutorOutcome::non_domain_failure(
                non_domain_failure(
                    NonDomainEntryStatus::MayHaveEntered,
                    NonDomainDisposition::IntegrityBlocked,
                    NonDomainFailureCode::AdapterContractViolation,
                )?,
            )?,
        },
    )
    .map_err(Into::into)
}

pub(crate) struct RoutedReadRequest {
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
        Box<dyn Any + Send + Sync>,
        ProposedValueMaterial,
        ContentRef,
    ) {
        (self.request, self.proposed, self.routing_generation_ref)
    }
}

pub(crate) enum ErasedReadOutcome {
    Returned(Box<dyn Any + Send + Sync>),
    DidNotEnter {
        diagnostic: Option<Box<dyn Any + Send + Sync>>,
        metadata: mfm_store::SafeFailureMetadata,
    },
    Indeterminate {
        diagnostic: Option<Box<dyn Any + Send + Sync>>,
        metadata: mfm_store::SafeFailureMetadata,
    },
    NonDomainFailure(NonDomainFailure),
}

pub(crate) struct ErasedReadObservation {
    pub(crate) authorization_ref: AuthorizationRef,
    pub(crate) outcome: ErasedReadOutcome,
}

pub(crate) struct ReadInvocationContext {
    pub(crate) routing_generation_ref: ContentRef,
    pub(crate) safe_failure_contract_ref: ContentRef,
    pub(crate) adapter_may_have_entered: NonDomainFailure,
}

type ErasedReadFuture<'a> = Pin<Box<dyn Future<Output = ErasedReadObservation> + Send + 'a>>;

trait PreparedErasedReadInvocation: Send {
    fn invoke_and_totalize(
        self: Box<Self>,
        witness: mfm_store::NewlyAppendedAuthorization,
    ) -> ErasedReadFuture<'static>;
}

pub(crate) struct PreparedReadInvocation {
    invocation: Box<dyn PreparedErasedReadInvocation>,
}

impl PreparedReadInvocation {
    fn new(invocation: Box<dyn PreparedErasedReadInvocation>) -> Self {
        Self { invocation }
    }

    pub(crate) fn invoke_and_totalize(
        self,
        witness: mfm_store::NewlyAppendedAuthorization,
    ) -> ErasedReadFuture<'static> {
        self.invocation.invoke_and_totalize(witness)
    }
}

trait ErasedReadCapability: Send + Sync {
    fn route_request(
        &self,
        entry: &QualifiedReadEntry,
        request: QualifiedAuthoredRequest,
    ) -> Result<RoutedReadRequest>;

    fn prepare_call(
        &self,
        expected_request_ref: ValueRef,
        context: ReadInvocationContext,
        request: RoutedReadRequest,
    ) -> Result<PreparedReadInvocation>;
}

struct TypedReadCapability<Request, Capability> {
    capability: Arc<Capability>,
    _request: PhantomData<fn(Request) -> Request>,
}

struct TypedPreparedReadInvocation<Request, Capability>
where
    Capability: AuditedReadCapability<Request>,
{
    capability: Arc<Capability>,
    access: PreparedReadAccess<Request>,
    safe_failure_contract_ref: ContentRef,
    adapter_may_have_entered: NonDomainFailure,
}

impl<Request, Capability> PreparedErasedReadInvocation
    for TypedPreparedReadInvocation<Request, Capability>
where
    Request: Send + Sync + 'static,
    Capability: AuditedReadCapability<Request>,
{
    fn invoke_and_totalize(
        self: Box<Self>,
        witness: mfm_store::NewlyAppendedAuthorization,
    ) -> ErasedReadFuture<'static> {
        Box::pin(async move {
            let authorization_ref = witness.authorization_ref().clone();
            let access = self.access.authorize(witness);
            let outcome = match self.capability.call(access).await {
                ReadCapabilityOutcome::Returned(value) => {
                    ErasedReadOutcome::Returned(Box::new(value))
                }
                ReadCapabilityOutcome::DidNotEnter {
                    diagnostic,
                    metadata,
                } if metadata.safe_failure_contract_ref() == &self.safe_failure_contract_ref => {
                    ErasedReadOutcome::DidNotEnter {
                        diagnostic: diagnostic
                            .map(|value| Box::new(value) as Box<dyn Any + Send + Sync>),
                        metadata,
                    }
                }
                ReadCapabilityOutcome::Indeterminate {
                    diagnostic,
                    metadata,
                } if metadata.safe_failure_contract_ref() == &self.safe_failure_contract_ref => {
                    ErasedReadOutcome::Indeterminate {
                        diagnostic: diagnostic
                            .map(|value| Box::new(value) as Box<dyn Any + Send + Sync>),
                        metadata,
                    }
                }
                ReadCapabilityOutcome::DidNotEnter { .. }
                | ReadCapabilityOutcome::Indeterminate { .. } => {
                    ErasedReadOutcome::NonDomainFailure(self.adapter_may_have_entered)
                }
            };
            ErasedReadObservation {
                authorization_ref,
                outcome,
            }
        })
    }
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
            || entry.diagnostic_type() != TypeId::of::<Capability::SafeDiagnostic>()
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
            request,
            proposed,
            routing_generation_ref,
        })
    }

    fn prepare_call(
        &self,
        expected_request_ref: ValueRef,
        context: ReadInvocationContext,
        request: RoutedReadRequest,
    ) -> Result<PreparedReadInvocation> {
        let (request, _, routed_generation_ref) = request.into_parts();
        if context.routing_generation_ref != routed_generation_ref {
            return Err(RuntimeError::CatalogSelection);
        }
        let request = request
            .downcast::<Request>()
            .map_err(|_| RuntimeError::InvalidCallbackResult)?;
        Ok(PreparedReadInvocation::new(Box::new(
            TypedPreparedReadInvocation::<Request, Capability> {
                capability: Arc::clone(&self.capability),
                access: PreparedReadAccess::new(expected_request_ref, *request),
                safe_failure_contract_ref: context.safe_failure_contract_ref,
                adapter_may_have_entered: context.adapter_may_have_entered,
            },
        )))
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
    pub(crate) effect_key: EffectKey,
    pub(crate) request_digest: RequestDigest,
    pub(crate) adapter_may_have_entered: EffectExecutorOutcome,
}

pub(crate) struct ErasedEnsureObservation {
    pub(crate) authorization_ref: AuthorizationRef,
    pub(crate) outcome: EffectExecutorOutcome,
}

type ErasedEnsureFuture<'a> = Pin<Box<dyn Future<Output = ErasedEnsureObservation> + Send + 'a>>;

trait PreparedErasedEffectInvocation: Send {
    fn invoke_and_totalize(
        self: Box<Self>,
        witness: mfm_store::NewlyAppendedAuthorization,
    ) -> ErasedEnsureFuture<'static>;
}

pub(crate) struct PreparedEffectInvocation {
    invocation: Box<dyn PreparedErasedEffectInvocation>,
}

impl PreparedEffectInvocation {
    fn new(invocation: Box<dyn PreparedErasedEffectInvocation>) -> Self {
        Self { invocation }
    }

    pub(crate) fn invoke_and_totalize(
        self,
        witness: mfm_store::NewlyAppendedAuthorization,
    ) -> ErasedEnsureFuture<'static> {
        self.invocation.invoke_and_totalize(witness)
    }
}

trait ErasedEffectExecutor: Send + Sync {
    fn identify_request(
        &self,
        context: EffectRequestContext,
        request: QualifiedAuthoredRequest,
    ) -> Result<PreparedEffectRequest>;

    fn prepare_ensure(
        &self,
        expected_request_ref: ValueRef,
        context: EffectInvocationContext,
        request: QualifiedAuthoredRequest,
    ) -> Result<PreparedEffectInvocation>;
}

struct TypedEffectExecutor<Request, Executor> {
    executor: Arc<Executor>,
    _request: PhantomData<fn(Request) -> Request>,
}

struct TypedPreparedEffectInvocation<Request, Executor>
where
    Request: CanonicalExecutorRequest + Send + Sync + 'static,
    Executor: RecoverableEffectExecutor<Request>,
{
    executor: Arc<Executor>,
    access: PreparedEnsureAccess<Request>,
    binding: VerifiedExecutorBinding,
    adapter_may_have_entered: EffectExecutorOutcome,
}

impl<Request, Executor> PreparedErasedEffectInvocation
    for TypedPreparedEffectInvocation<Request, Executor>
where
    Request: CanonicalExecutorRequest + Send + Sync + 'static,
    Executor: RecoverableEffectExecutor<Request>,
{
    fn invoke_and_totalize(
        self: Box<Self>,
        witness: mfm_store::NewlyAppendedAuthorization,
    ) -> ErasedEnsureFuture<'static> {
        Box::pin(async move {
            let authorization_ref = witness.authorization_ref().clone();
            let access = self.access.authorize(witness);
            let outcome = self.executor.ensure(access).await;
            let valid = match outcome.view() {
                EffectExecutorOutcomeView::Returned(result) => {
                    result.identity().executor_binding_ref() == self.binding.binding_ref()
                        && result.retained_closure().executor_binding_ref()
                            == self.binding.binding_ref()
                        && result.retained_closure().contract()
                            == self.binding.contract().retained_closure_contract()
                }
                EffectExecutorOutcomeView::DidNotEnter(failure)
                | EffectExecutorOutcomeView::Indeterminate(failure) => {
                    failure.safe_failure_contract_ref()
                        == self.binding.contract().safe_failure_contract_ref()
                }
                EffectExecutorOutcomeView::NonDomainFailure(failure) => failure
                    .validate_layer(mfm_journal::v2::NonDomainFailureLayer::Ensure)
                    .is_ok(),
            };
            ErasedEnsureObservation {
                authorization_ref,
                outcome: if valid {
                    outcome
                } else {
                    self.adapter_may_have_entered
                },
            }
        })
    }
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

    fn prepare_ensure(
        &self,
        expected_request_ref: ValueRef,
        context: EffectInvocationContext,
        request: QualifiedAuthoredRequest,
    ) -> Result<PreparedEffectInvocation> {
        if request.request_type() != TypeId::of::<Request>()
            || context.request_type != TypeId::of::<Request>()
            || context.response_type != TypeId::of::<VerifiedEnsureResult>()
            || context.failure_type != TypeId::of::<mfm_executor::ExecutorError>()
            || context.operation_id.is_empty()
            || context.tenant_scope_id != *context.binding.deployment().tenant_scope_id()
        {
            return Err(RuntimeError::EffectIdentityMismatch);
        }
        expected_request_ref
            .validate_contract(context.binding.contract().semantic_request_contract())
            .map_err(|_| RuntimeError::EffectIdentityMismatch)?;
        let request = request
            .into_value()
            .downcast::<Request>()
            .map_err(|_| RuntimeError::InvalidCallbackResult)?;
        let expected = expected_request_ref.fields()?;
        let canonical = request.canonical_request();
        let content_ref = canonical.reference()?;
        let byte_length = u64::try_from(canonical.as_bytes().len())
            .map_err(|_| RuntimeError::EffectIdentityMismatch)?;
        if &expected.schema_id != content_ref.schema_id()
            || &expected.content_digest != content_ref.content_digest()
            || expected.byte_length != byte_length
        {
            return Err(RuntimeError::EffectIdentityMismatch);
        }
        let committed = CommittedEffectRequest::new(
            context.binding.binding_ref().clone(),
            context.tenant_scope_id.clone(),
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
        Ok(PreparedEffectInvocation::new(Box::new(
            TypedPreparedEffectInvocation::<Request, Executor> {
                executor: Arc::clone(&self.executor),
                access: PreparedEnsureAccess::new(expected_request_ref, committed),
                binding: context.binding,
                adapter_may_have_entered: context.adapter_may_have_entered,
            },
        )))
    }
}

#[derive(Clone)]
pub(crate) struct RuntimeReadInvoker {
    erased: Arc<dyn ErasedReadCapability>,
    adapter_may_have_entered: NonDomainFailure,
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

    pub(crate) fn adapter_may_have_entered(&self) -> NonDomainFailure {
        self.adapter_may_have_entered
    }

    pub(crate) fn prepare_call(
        &self,
        expected_request_ref: ValueRef,
        context: ReadInvocationContext,
        request: RoutedReadRequest,
    ) -> Result<PreparedReadInvocation> {
        self.erased
            .prepare_call(expected_request_ref, context, request)
    }
}

#[derive(Clone)]
pub(crate) struct RuntimeEffectInvoker {
    erased: Arc<dyn ErasedEffectExecutor>,
    adapter_may_have_entered: EffectExecutorOutcome,
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

    pub(crate) fn adapter_may_have_entered(&self) -> EffectExecutorOutcome {
        self.adapter_may_have_entered.clone()
    }

    pub(crate) fn prepare_ensure(
        &self,
        expected_request_ref: ValueRef,
        context: EffectInvocationContext,
        request: QualifiedAuthoredRequest,
    ) -> Result<PreparedEffectInvocation> {
        self.erased
            .prepare_ensure(expected_request_ref, context, request)
    }
}
