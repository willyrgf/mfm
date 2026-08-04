//! Direct structured balance adapters over the bounded exact EVM transport.

use std::marker::PhantomData;
use std::sync::Arc;

use mfm_capabilities::{
    AccessFaultCode, ComponentFuture, ReadAdapterCompletion, ReadAdapterInvoker,
};
use mfm_certify::structured::CertifiedAccessAuthorization;
use mfm_certify::structured::{
    PhysicalBindingSelection, ProgramRegistryBuilder, QualifiedReadPhysicalBinding,
    QualifiedReadPhysicalBindingSource,
};
use mfm_evm::{
    balance_adapter_contract, EvmAnchorConfirmationRequest, EvmBlockResponse,
    EvmChainIdentityCapability, EvmChainIdentityRequest, EvmChainIdentityResponse,
    EvmConfirmAnchorCapability, EvmLatestAnchorCapability, EvmLatestAnchorRequest,
    EvmNativeBalanceCapability, EvmNativeBalanceRequest, EvmQuantityResponse, EvmReadFailure,
    EvmSafeFailure, EvmSubmissionProcessQualification, EvmTokenBalanceCapability,
    EvmTokenBalanceRequest, EvmTokenDecimalsCapability, EvmTokenDecimalsRequest,
    EvmTokenDecimalsResponse,
};
use mfm_ids::{ContentRef, StableId};
use mfm_journal::structured::{AccessKind, HistoryObject};
use mfm_program::structured::{RuntimeReadAdapter, RuntimeReadCapability};
use mfm_spec::structured::{SecretFreeImplementationDescriptor, StructuredComponentKind};

use crate::structured::AuthorizedProviderCall;
use crate::transport::{EvmJsonRpcTransport, EvmTransportOutcome};
use crate::{
    EvmPhysicalBindingPurpose, EvmPhysicalBindingReleaseHistory, EvmStructuredLiveBindingError,
};

/// Qualified exact-route source for all six structured balance reads.
#[derive(Clone)]
pub struct EvmStructuredBalanceBindings {
    transport: Arc<EvmJsonRpcTransport>,
    admitted_routing_policy_ref: ContentRef,
    release_history: EvmPhysicalBindingReleaseHistory,
    integrity_fault: AccessFaultCode,
}

impl EvmStructuredBalanceBindings {
    /// Binds one exact transport catalog and complete retained release history.
    pub fn new(
        transport: Arc<EvmJsonRpcTransport>,
        admitted_routing_policy_ref: ContentRef,
        release_history: EvmPhysicalBindingReleaseHistory,
    ) -> Result<Self, EvmStructuredLiveBindingError> {
        let physical_target_ref = transport
            .routing_catalog_descriptor()
            .content_ref()
            .map_err(|_| EvmStructuredLiveBindingError::InvalidContract)?;
        if release_history.current().admitted_routing_policy_ref() != &admitted_routing_policy_ref
            || release_history.current().physical_target_ref() != &physical_target_ref
        {
            return Err(EvmStructuredLiveBindingError::InvalidContract);
        }
        Ok(Self {
            transport,
            admitted_routing_policy_ref,
            release_history,
            integrity_fault: AccessFaultCode::new(
                StableId::new("mfm.evm-live/structured-balance-integrity-fault")
                    .map_err(|_| EvmStructuredLiveBindingError::InvalidContract)?,
            ),
        })
    }

    fn selected(&self, selection: PhysicalBindingSelection<'_>) -> bool {
        selection.admitted_routing_policy_ref() == &self.admitted_routing_policy_ref
    }

    /// Returns the immutable admitted routing policy selected by this binding.
    pub const fn admitted_routing_policy_ref(&self) -> &ContentRef {
        &self.admitted_routing_policy_ref
    }

    /// Returns the complete secret-free catalog bound by the transport.
    pub fn routing_catalog_descriptor(&self) -> &mfm_evm::EvmRoutingCatalogDescriptor {
        self.transport.routing_catalog_descriptor()
    }

    /// Returns the exact secret-free physical certificate retained in history.
    pub fn public_certificate(&self) -> &HistoryObject {
        self.release_history.current().certificate()
    }

    /// Returns every retained physical release for the balance transport.
    pub const fn release_history(&self) -> &EvmPhysicalBindingReleaseHistory {
        &self.release_history
    }
}

trait BalanceReadSpec: RuntimeReadCapability<SafeFailure = EvmReadFailure> {
    fn adapter_id() -> &'static str;

    fn invoke<'a>(
        transport: &'a EvmJsonRpcTransport,
        request: &'a Self::Request,
        authorization: CertifiedAccessAuthorization,
    ) -> ComponentFuture<'a, EvmTransportOutcome<Self::Returned>>;
}

macro_rules! read_spec {
    ($capability:ty, $request:ty, $returned:ty, $method:ident, $adapter:literal) => {
        impl BalanceReadSpec for $capability {
            fn adapter_id() -> &'static str {
                $adapter
            }

            fn invoke<'a>(
                transport: &'a EvmJsonRpcTransport,
                request: &'a $request,
                authorization: CertifiedAccessAuthorization,
            ) -> ComponentFuture<'a, EvmTransportOutcome<$returned>> {
                let provider_call = AuthorizedProviderCall::new(&authorization);
                let origin = provider_call.origin().clone();
                Box::pin(async move {
                    provider_call
                        .run(transport.run_authorized(&origin, transport.$method(request)))
                        .await
                })
            }
        }
    };
}

read_spec!(
    EvmChainIdentityCapability,
    EvmChainIdentityRequest,
    EvmChainIdentityResponse,
    chain_identity,
    "mfm.evm.adapter/chain-identity"
);
read_spec!(
    EvmLatestAnchorCapability,
    EvmLatestAnchorRequest,
    EvmBlockResponse,
    latest_anchor,
    "mfm.evm.adapter/latest-anchor"
);
read_spec!(
    EvmTokenDecimalsCapability,
    EvmTokenDecimalsRequest,
    EvmTokenDecimalsResponse,
    token_decimals,
    "mfm.evm.adapter/token-decimals"
);
read_spec!(
    EvmNativeBalanceCapability,
    EvmNativeBalanceRequest,
    EvmQuantityResponse,
    native_balance,
    "mfm.evm.adapter/native-balance"
);
read_spec!(
    EvmTokenBalanceCapability,
    EvmTokenBalanceRequest,
    EvmQuantityResponse,
    token_balance,
    "mfm.evm.adapter/token-balance"
);
read_spec!(
    EvmConfirmAnchorCapability,
    EvmAnchorConfirmationRequest,
    EvmBlockResponse,
    confirm_anchor,
    "mfm.evm.adapter/confirm-anchor"
);

/// Opaque current binding for one exact structured balance capability.
#[doc(hidden)]
pub struct EvmStructuredBalanceReadBinding<C> {
    source: EvmStructuredBalanceBindings,
    _capability: PhantomData<fn() -> C>,
}

impl<C> ReadAdapterInvoker<C> for EvmStructuredBalanceReadBinding<C>
where
    C: BalanceReadSpec,
{
    fn invoke<'a>(
        &'a self,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<C::Returned, C::SafeFailure>> {
        let _ = request;
        Box::pin(std::future::ready(ReadAdapterCompletion::IntegrityFault(
            self.source.integrity_fault.clone(),
        )))
    }
}

impl<C> RuntimeReadAdapter<C> for EvmStructuredBalanceReadBinding<C>
where
    C: BalanceReadSpec,
{
    fn contract() -> mfm_program::Result<mfm_spec::structured::StructuredLiveComponentContract> {
        balance_adapter_contract(C::adapter_id())
    }
}

impl<C> QualifiedReadPhysicalBinding<C> for EvmStructuredBalanceReadBinding<C>
where
    C: BalanceReadSpec,
{
    fn public_certificate(&self) -> &HistoryObject {
        self.source.release_history.current().certificate()
    }

    fn invoke_authorized<'a>(
        &'a self,
        request: &'a C::Request,
        authorization: CertifiedAccessAuthorization,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<C::Returned, C::SafeFailure>> {
        if authorization.authorization().access_kind != AccessKind::Read
            || authorization.authorization().physical_binding_ref
                != self.public_certificate().content_ref
        {
            return Box::pin(std::future::ready(ReadAdapterCompletion::IntegrityFault(
                self.source.integrity_fault.clone(),
            )));
        }
        let provider_call = AuthorizedProviderCall::new(&authorization);
        let origin = provider_call.origin().clone();
        Box::pin(async move {
            let outcome = provider_call
                .run(self.source.transport.run_authorized(
                    &origin,
                    C::invoke(self.source.transport.as_ref(), request, authorization),
                ))
                .await;
            match outcome {
                EvmTransportOutcome::Returned(returned) => {
                    ReadAdapterCompletion::Returned(returned)
                }
                EvmTransportOutcome::SafeFailure(failure) => {
                    classify_failure(failure, &self.source.integrity_fault)
                }
            }
        })
    }
}

impl<C> QualifiedReadPhysicalBindingSource<C> for EvmStructuredBalanceBindings
where
    C: BalanceReadSpec,
{
    type Binding = EvmStructuredBalanceReadBinding<C>;

    fn current_binding<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        _request: &'a C::Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        let binding = self.selected(selection).then(|| {
            Arc::new(EvmStructuredBalanceReadBinding {
                source: self.clone(),
                _capability: PhantomData,
            })
        });
        Box::pin(async move { binding })
    }
}

fn classify_failure<R>(
    failure: EvmSafeFailure,
    integrity_fault: &AccessFaultCode,
) -> ReadAdapterCompletion<R, EvmReadFailure> {
    match failure {
        EvmSafeFailure::RoutingGenerationUnavailable
        | EvmSafeFailure::ConfigurationInvalid
        | EvmSafeFailure::RequestInvalid
        | EvmSafeFailure::ResponseInvalid { .. }
        | EvmSafeFailure::ResponseMissingResult { .. }
        | EvmSafeFailure::ResponseTooLarge { .. } => {
            ReadAdapterCompletion::IntegrityFault(integrity_fault.clone())
        }
        EvmSafeFailure::HttpStatus { status }
            if !matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504 | 507) =>
        {
            ReadAdapterCompletion::SafeFailure(EvmReadFailure::DestinationRejected)
        }
        EvmSafeFailure::JsonRpcError { json_rpc_code }
            if !matches!(json_rpc_code, -32603 | -32001 | -32002 | -32005) =>
        {
            ReadAdapterCompletion::SafeFailure(EvmReadFailure::DestinationRejected)
        }
        EvmSafeFailure::AccessCancelled
        | EvmSafeFailure::TransportFailed
        | EvmSafeFailure::HttpStatus { .. }
        | EvmSafeFailure::JsonRpcError { .. }
        | EvmSafeFailure::UnclassifiedFailure => {
            ReadAdapterCompletion::SafeFailure(EvmReadFailure::Unavailable)
        }
    }
}

/// Registers all six direct structured balance adapters and returns their exact
/// physical purpose tuples.
pub fn register_evm_balance_bindings(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    bindings: Arc<EvmStructuredBalanceBindings>,
) -> mfm_certify::Result<Vec<EvmPhysicalBindingPurpose>> {
    Ok(vec![
        register_adapter::<EvmChainIdentityCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
        )?,
        register_adapter::<EvmLatestAnchorCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
        )?,
        register_adapter::<EvmTokenDecimalsCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
        )?,
        register_adapter::<EvmNativeBalanceCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
        )?,
        register_adapter::<EvmTokenBalanceCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
        )?,
        register_adapter::<EvmConfirmAnchorCapability>(registry, qualification, bindings)?,
    ])
}

fn register_adapter<C>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    bindings: Arc<EvmStructuredBalanceBindings>,
) -> mfm_certify::Result<EvmPhysicalBindingPurpose>
where
    C: BalanceReadSpec,
{
    let adapter_contract_ref =
        <EvmStructuredBalanceReadBinding<C> as RuntimeReadAdapter<C>>::contract()
            .and_then(|contract| contract.content_ref().map_err(Into::into))
            .map_err(certification_error)?;
    let capability_contract_ref = <C as RuntimeReadCapability>::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(certification_error)?;
    let descriptor = SecretFreeImplementationDescriptor {
        component_kind: StructuredComponentKind::Adapter,
        semantic_contract_ref: adapter_contract_ref.clone(),
        implementation_id: StableId::new(format!(
            "mfm.evm-live.implementation/structured-balance-{}",
            C::adapter_id().rsplit('/').next().unwrap_or("read")
        ))
        .map_err(certification_error)?,
        executable_identity_ref: qualification.executable_identity_ref().clone(),
        qualification_artifact_ref: qualification.qualification_artifact_ref().clone(),
    };
    let adapter_implementation_ref = descriptor.content_ref().map_err(certification_error)?;
    let purpose = EvmPhysicalBindingPurpose::new(
        AccessKind::Read,
        capability_contract_ref,
        adapter_contract_ref,
        adapter_implementation_ref,
        None,
        bindings.release_history.clone(),
    )
    .map_err(certification_error)?;
    registry.register_read_adapter::<C, _>(descriptor, bindings)?;
    Ok(purpose)
}

fn certification_error(error: impl std::fmt::Display) -> mfm_certify::CertifyError {
    mfm_certify::CertifyError::Certification(error.to_string())
}
