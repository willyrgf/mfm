//! Explicit native resources shared by fresh construction and cold association.
use std::{marker::PhantomData, sync::Arc};

use mfm_capabilities::{
    EffectAdapter, EffectAdapterOutcome, EffectImplementation, ReadAdapter, ReadImplementation,
};
use mfm_chain::{
    balance::{BalanceRead, BalanceSourceDefinition},
    transaction::{ContractRead, LifecyclePlanning, TransactionEffect},
};
use mfm_evm::custody::EvmTransactionAuthority;
use mfm_evm::*;
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_program::{BindEffect, BindRead, CapabilityFamily, ProgramEnvironment, Resolve};
use mfm_signing::Secp256k1Signer;
use mfm_values::{InvocationDiagnostic, Object};
use serde::Serialize;

use crate::{EvmReadProvider, EvmTransactionProvider, ProviderFuture};

/// Maximum number of native resources in one environment.
pub const MAX_EVM_BINDINGS: usize = 256;

/// Public route information derived from the resources used for binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvmBindingView {
    /// One native observational route.
    Evm {
        /// Public chain identifier.
        chain_id: u64,
        /// Public endpoint name, never a locator.
        endpoint_id: String,
        /// Exact physical target commitment.
        binding_ref: ContentRef,
    },
}

/// Checked transaction handles attached to one exact public binding.
#[derive(Clone)]
pub struct EvmTransactionResource {
    binding: EvmTransactionBinding,
    signer: Arc<dyn Secp256k1Signer>,
    authority: Arc<dyn EvmTransactionAuthority>,
    provider: Arc<dyn EvmTransactionProvider>,
}
impl EvmTransactionResource {
    /// Checks authority, signing purpose, and sender before handles can be installed.
    pub fn new(
        binding: EvmTransactionBinding,
        signer: Arc<dyn Secp256k1Signer>,
        authority: Arc<dyn EvmTransactionAuthority>,
        provider: Arc<dyn EvmTransactionProvider>,
    ) -> Result<Self, InvocationDiagnostic> {
        let purpose = StableId::new(crate::EVM_EIP1559_SIGNING_PURPOSE_ID)
            .map_err(|cause| diagnostic("signing_purpose", &cause))?;
        let sender = crate::ethereum_address(signer.public_key());
        if &binding.authority_epoch != authority.authority_epoch()
            || signer.purpose() != &purpose
            || sender != binding.sender
        {
            return Err(diagnostic(
                "transaction_resource",
                &serde_json::json!({
                    "reason": "resource_identity_mismatch", "binding": binding,
                    "authority_epoch": authority.authority_epoch(),
                    "signing_purpose": signer.purpose(), "sender": sender,
                }),
            ));
        }
        Ok(Self {
            binding,
            signer,
            authority,
            provider,
        })
    }
}

/// Native binding environment with a type-only installed source inventory.
/// Public facts and live handles have one owner; inspection derives its views from these records.
pub struct EvmResources<Sources> {
    reads: Vec<(EvmBalanceRoute, Arc<dyn EvmReadProvider>)>,
    transactions: Vec<EvmTransactionResource>,
    sources: PhantomData<fn() -> Sources>,
}
impl<Sources> ProgramEnvironment for EvmResources<Sources> {
    type Sources = Sources;
}
impl<Sources> EvmResources<Sources> {
    /// Admits strictly ordered observational routes and unique transaction bindings without IO.
    pub fn new(
        reads: Vec<(EvmBalanceRoute, Arc<dyn EvmReadProvider>)>,
        transactions: Vec<EvmTransactionResource>,
    ) -> Result<Self, InvocationDiagnostic> {
        if reads.len().saturating_add(transactions.len()) > MAX_EVM_BINDINGS {
            return Err(diagnostic(
                "admit_resources",
                &serde_json::json!({
                    "reason": "resource_limit", "reads": reads.len(), "transactions": transactions.len(),
                    "maximum": MAX_EVM_BINDINGS,
                }),
            ));
        }
        for (position, pair) in reads.windows(2).enumerate() {
            let key = |route: &EvmBalanceRoute| {
                (route.chain_id(), route.endpoint().endpoint_id().to_owned())
            };
            if key(&pair[0].0) >= key(&pair[1].0) {
                return Err(diagnostic(
                    "admit_resources",
                    &serde_json::json!({
                        "reason": "routes_not_strictly_ordered", "position": position + 1,
                        "previous": pair[0].0, "current": pair[1].0,
                    }),
                ));
            }
        }
        for (position, resource) in transactions.iter().enumerate() {
            if transactions[..position]
                .iter()
                .any(|earlier| earlier.binding == resource.binding)
            {
                return Err(diagnostic(
                    "admit_resources",
                    &serde_json::json!({
                        "reason": "duplicate_transaction_binding", "position": position, "binding": resource.binding,
                    }),
                ));
            }
        }
        Ok(Self {
            reads,
            transactions,
            sources: PhantomData,
        })
    }

    /// Derives public observational binding views without a second binding cache.
    pub fn bindings(&self) -> Result<Vec<EvmBindingView>, InvocationDiagnostic> {
        self.reads
            .iter()
            .map(|(route, _)| {
                Ok(EvmBindingView::Evm {
                    chain_id: route.chain_id().get(),
                    endpoint_id: route.endpoint().endpoint_id().to_owned(),
                    binding_ref: route
                        .route_ref()
                        .map_err(|cause| cause.into_diagnostic("inspect_route"))?,
                })
            })
            .collect()
    }

    fn read_provider(
        &self,
        target: &EvmPhysicalTarget,
    ) -> Result<Arc<dyn EvmReadProvider>, InvocationDiagnostic> {
        for (route, provider) in &self.reads {
            let installed = route
                .physical_target()
                .map_err(|cause| cause.into_diagnostic("bind_read"))?;
            if &installed == target {
                return Ok(provider.clone());
            }
        }
        Err(diagnostic(
            "bind_read",
            &serde_json::json!({"reason": "route_not_installed", "target": target}),
        ))
    }

    fn balance(
        &self,
        binding: &EvmBalanceBinding,
        family: ReadCapabilityFamily,
    ) -> Result<EvmBalanceAdapter, InvocationDiagnostic> {
        let provider = self.read_provider(binding.route())?;
        let reference = Object::from_value(binding.route())
            .map_err(|cause| cause.into_diagnostic("bind_read"))?
            .value_ref()
            .clone();
        Ok(EvmBalanceAdapter {
            target: binding.route().clone(),
            reference,
            provider,
            family,
        })
    }

    fn transaction(
        &self,
        binding: &EvmTransactionBinding,
    ) -> Result<EvmTransactionResource, InvocationDiagnostic> {
        self.transactions.iter().find(|resource| &resource.binding == binding).cloned()
            .ok_or_else(|| diagnostic("bind_effect", &serde_json::json!({"reason": "transaction_binding_not_installed", "binding": binding})))
    }
}

fn diagnostic(operation: &'static str, fields: &impl Serialize) -> InvocationDiagnostic {
    InvocationDiagnostic::from_fields("native_binding", operation, fields, None)
}

impl<S> CapabilityFamily<BalanceRead> for EvmResources<S> {
    type Implementations = (EvmNativeBalance, EvmTokenBalance);
}
impl<S, K> Resolve<BalanceSourceDefinition<K>, BalanceRead> for EvmResources<S> {
    fn implementation(config: &BalanceSourceDefinition<K>) -> mfm_program::Result<StableId> {
        let target = config
            .source()
            .target()
            .native()
            .decode::<EvmBalanceTarget>()
            .map_err(mfm_program::ProgramError::Diagnostic)?;
        Ok(if target.token().is_some() {
            EvmTokenBalance::implementation_id()?
        } else {
            EvmNativeBalance::implementation_id()?
        })
    }
}
impl<S, R: EvmTransactionRecipe> CapabilityFamily<TransactionEffect<R>> for EvmResources<S> {
    type Implementations = (EvmTransactionImplementation,);
}
impl<S> CapabilityFamily<ContractRead> for EvmResources<S> {
    type Implementations = (EvmContractReadImplementation,);
}
impl<S, C: LifecyclePlanning + ?Sized, R: EvmTransactionRecipe> Resolve<C, TransactionEffect<R>>
    for EvmResources<S>
{
    fn implementation(config: &C) -> mfm_program::Result<StableId> {
        Ok(config
            .deployment_request()
            .execution()
            .transaction_implementation()
            .clone())
    }
}
impl<S, C: LifecyclePlanning + ?Sized> Resolve<C, ContractRead> for EvmResources<S> {
    fn implementation(config: &C) -> mfm_program::Result<StableId> {
        Ok(config
            .deployment_request()
            .execution()
            .read_implementation()
            .clone())
    }
}

/// Bound native balance observer, constructed only by exact resource matching.
pub struct EvmBalanceAdapter {
    target: EvmPhysicalTarget,
    reference: ContentRef,
    provider: Arc<dyn EvmReadProvider>,
    family: ReadCapabilityFamily,
}
impl ReadAdapter<EvmReadIntent, EvmReadEvidence, EvmOperationalError> for EvmBalanceAdapter {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        native_ref: &'a ContentRef,
        intent: &'a EvmReadIntent,
    ) -> ProviderFuture<'a, EvmReadEvidence> {
        Box::pin(crate::read(
            &self.target,
            &self.reference,
            self.provider.as_ref(),
            self.family,
            native_ref,
            intent,
        ))
    }
}
macro_rules! balance_binding {
    ($capability:ty, $implementation:ty, $family:ident) => {
        impl<S> BindRead<$capability, $implementation> for EvmResources<S> {
            type Adapter = EvmBalanceAdapter;
            fn bind_read(
                &self,
                binding: &EvmBalanceBinding,
            ) -> Result<Self::Adapter, InvocationDiagnostic> {
                self.balance(binding, ReadCapabilityFamily::$family)
            }
        }
    };
}
balance_binding!(BalanceRead, EvmNativeBalance, Balance);
balance_binding!(BalanceRead, EvmTokenBalance, Balance);
balance_binding!(EvmChainIdentityRead, EvmBalanceObservation, ChainIdentity);
balance_binding!(EvmAnchorRead, EvmBalanceObservation, Anchor);
balance_binding!(EvmBalanceRead, EvmBalanceObservation, Balance);

/// Bound anchored contract observer, constructed only by exact resource matching.
pub struct EvmContractAdapter {
    route: EvmTransactionRoute,
    reference: ContentRef,
    provider: Arc<dyn EvmReadProvider>,
}
impl<S> BindRead<ContractRead, EvmContractReadImplementation> for EvmResources<S> {
    type Adapter = EvmContractAdapter;
    fn bind_read(
        &self,
        route: &EvmTransactionRoute,
    ) -> Result<Self::Adapter, InvocationDiagnostic> {
        let provider = self.read_provider(&EvmPhysicalTarget {
            chain_id: route.chain_instance.chain_id,
            endpoint_ref: route.endpoint_ref.clone(),
        })?;
        let reference = Object::from_value(route)
            .map_err(|cause| cause.into_diagnostic("bind_contract_read"))?
            .value_ref()
            .clone();
        Ok(EvmContractAdapter {
            route: route.clone(),
            reference,
            provider,
        })
    }
}
impl ReadAdapter<AnchoredContractCallIntent, AnchoredContractCallEvidence, EvmOperationalError>
    for EvmContractAdapter
{
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        native_ref: &'a ContentRef,
        intent: &'a AnchoredContractCallIntent,
    ) -> ProviderFuture<'a, AnchoredContractCallEvidence> {
        Box::pin(crate::read_anchored(
            &self.route,
            &self.reference,
            self.provider.as_ref(),
            native_ref,
            intent,
        ))
    }
}

impl<S, C, I> BindEffect<C, I> for EvmResources<S>
where
    C: mfm_capabilities::EffectCapabilityContract,
    I: EffectImplementation<C, Binding = EvmTransactionBinding>,
    EvmTransactionResource: EffectAdapter<I::NativeCommand, I::NativeEvidence, I::OperationalError>,
{
    type Adapter = EvmTransactionResource;
    fn bind_effect(
        &self,
        binding: &EvmTransactionBinding,
    ) -> Result<Self::Adapter, InvocationDiagnostic> {
        self.transaction(binding)
    }
}

type EffectFuture<'a, T> = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<
                    EffectAdapterOutcome<T>,
                    mfm_capabilities::AdapterError<EvmTransactionOperationalError>,
                >,
            > + Send
            + 'a,
    >,
>;
impl EffectAdapter<Eip1559TransactionCommand, Reservation, EvmTransactionOperationalError>
    for EvmTransactionResource
{
    fn invoke<'a>(
        &'a self,
        effect: &'a EffectId,
        _: &'a ContentRef,
        native_ref: &'a ContentRef,
        command: &'a Eip1559TransactionCommand,
    ) -> EffectFuture<'a, Reservation> {
        Box::pin(crate::transaction::reserve_nonce(
            &self.binding,
            self.authority.as_ref(),
            self.provider.as_ref(),
            effect,
            native_ref,
            command,
        ))
    }
}
impl
    EffectAdapter<
        ReservedEvmTransaction,
        PreparedEvmTransactionEvidence,
        EvmTransactionOperationalError,
    > for EvmTransactionResource
{
    fn invoke<'a>(
        &'a self,
        effect: &'a EffectId,
        _: &'a ContentRef,
        _: &'a ContentRef,
        command: &'a ReservedEvmTransaction,
    ) -> EffectFuture<'a, PreparedEvmTransactionEvidence> {
        Box::pin(crate::transaction::prepare_transaction(
            &self.binding,
            self.authority.as_ref(),
            self.signer.as_ref(),
            effect,
            command,
        ))
    }
}
impl EffectAdapter<PreparedEvmTransaction, EvmTransactionSettlement, EvmTransactionOperationalError>
    for EvmTransactionResource
{
    fn invoke<'a>(
        &'a self,
        effect: &'a EffectId,
        _: &'a ContentRef,
        _: &'a ContentRef,
        command: &'a PreparedEvmTransaction,
    ) -> EffectFuture<'a, EvmTransactionSettlement> {
        Box::pin(crate::transaction::execute_transaction(
            &self.binding,
            self.authority.as_ref(),
            self.provider.as_ref(),
            effect,
            command,
        ))
    }
}
