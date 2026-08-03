//! Target-bound structured adapters for the narrow wallet nonce authority.

use std::marker::PhantomData;
use std::sync::Arc;

use mfm_capabilities::{
    BoundedComponentInvoker, ComponentFuture, EffectAdapterInvoker, EffectRefreshMode,
    ReadAdapterCompletion, ReadAdapterInvoker,
};
use mfm_certify::structured::{
    PhysicalBindingSelection, ProgramRegistryBuilder, RuntimeEffectPhysicalBinding,
    RuntimeEffectPhysicalBindingSource, RuntimeReadPhysicalBinding,
    RuntimeReadPhysicalBindingSource,
};
use mfm_evm::{
    activate_wallet_candidate_adapter_contract, complete_wallet_nonce_adapter_contract,
    read_wallet_nonce_status_adapter_contract, reserve_wallet_nonce_adapter_contract,
    ActivateEvmCandidateRequest, ActivateWalletCandidateCapability, CompleteEvmNonceRequest,
    CompleteWalletNonceCapability, EvmSubmissionFailure, EvmSubmissionProcessQualification,
    ReadEvmWalletNonceStatusRequest, ReadWalletNonceStatusCapability, ReserveEvmNonceRequest,
    ReserveWalletNonceCapability, WalletNonceAuthority, WalletNonceAuthorityResource,
    WalletNonceDomainActivationAttestation, WalletNonceStatus,
};
use mfm_ids::{ContentRef, StableId};
use mfm_journal::structured::{AccessKind, HistoryObject, LexicalValueRef};
use mfm_program::structured::{
    RuntimeEffectAdapter, RuntimeEffectCapability, RuntimeReadAdapter, RuntimeReadCapability,
    RuntimeResourceAuthority,
};
use mfm_spec::structured::{
    SecretFreeImplementationDescriptor, StructuredComponentKind, StructuredLiveComponentContract,
};

use crate::{
    EvmPhysicalBindingPurpose, EvmPhysicalBindingReleaseHistory, EvmStructuredLiveBindingError,
};

/// One sealed target-bound wallet authority and its public certificates.
#[derive(Clone)]
pub struct EvmStructuredWalletBindings {
    authority: Arc<dyn WalletNonceAuthority>,
    domain_activation_attestation: WalletNonceDomainActivationAttestation,
    admitted_routing_policy_ref: ContentRef,
    resource_contract_ref: ContentRef,
    read_release_history: EvmPhysicalBindingReleaseHistory,
    effect_release_history: EvmPhysicalBindingReleaseHistory,
}

impl EvmStructuredWalletBindings {
    /// Binds one current wallet authority and complete retained release histories
    /// without exposing its database pool, fence session, or mutation capability.
    pub fn new(
        authority: Arc<dyn WalletNonceAuthority>,
        admitted_routing_policy_ref: ContentRef,
        read_release_history: EvmPhysicalBindingReleaseHistory,
        effect_release_history: EvmPhysicalBindingReleaseHistory,
    ) -> Result<Self, EvmStructuredLiveBindingError> {
        let domain_activation_attestation = authority.domain_activation_attestation().clone();
        let initial_target_ref = domain_activation_attestation
            .initial_store_incarnation_ref
            .to_content_ref()
            .map_err(|_| EvmStructuredLiveBindingError::InvalidContract)?;
        let resource_contract_ref = WalletNonceAuthorityResource::contract()
            .and_then(|contract| contract.content_ref().map_err(Into::into))
            .map_err(|_| EvmStructuredLiveBindingError::InvalidContract)?;
        // Every retained release (root and successors) must name the authority's
        // actual current physical incarnation. A successor that claims a rotated
        // target while the concrete authority still represents another
        // incarnation is rejected at construction and rechecked per access.
        let releases_match_authority = read_release_history
            .releases()
            .chain(effect_release_history.releases())
            .all(|release| release.physical_target_ref() == &initial_target_ref);
        if domain_activation_attestation.validate().is_err()
            || read_release_history.current().admitted_routing_policy_ref()
                != &admitted_routing_policy_ref
            || effect_release_history
                .current()
                .admitted_routing_policy_ref()
                != &admitted_routing_policy_ref
            || !releases_match_authority
            || !authority_release_is_current(
                authority.as_ref(),
                read_release_history.current().physical_target_ref(),
            )
            || !authority_release_is_current(
                authority.as_ref(),
                effect_release_history.current().physical_target_ref(),
            )
        {
            return Err(EvmStructuredLiveBindingError::InvalidContract);
        }
        Ok(Self {
            authority,
            domain_activation_attestation,
            admitted_routing_policy_ref,
            resource_contract_ref,
            read_release_history,
            effect_release_history,
        })
    }

    /// Consumes one physical-release currentness permit for this access.
    ///
    /// Returns false when the current release no longer matches the authority's
    /// exact physical incarnation or the admitted routing policy. Callers must
    /// revalidate immediately before every protected side effect.
    fn consume_physical_release_permit(&self, effect: bool) -> bool {
        let history = if effect {
            &self.effect_release_history
        } else {
            &self.read_release_history
        };
        let current = history.current();
        current.admitted_routing_policy_ref() == &self.admitted_routing_policy_ref
            && authority_release_is_current(self.authority.as_ref(), current.physical_target_ref())
            && history
                .releases()
                .all(|release| release.physical_target_ref() == current.physical_target_ref())
    }

    fn read_selection(&self, selection: PhysicalBindingSelection<'_>) -> bool {
        selection.admitted_routing_policy_ref == &self.admitted_routing_policy_ref
            && self.consume_physical_release_permit(false)
    }

    fn effect_selection(&self, selection: PhysicalBindingSelection<'_>) -> bool {
        self.read_selection(selection)
            && selection.stable_resource_lineage_contract_ref == Some(&self.resource_contract_ref)
            && self.consume_physical_release_permit(true)
    }

    /// Returns the immutable admitted routing policy selected by this authority.
    pub const fn admitted_routing_policy_ref(&self) -> &ContentRef {
        &self.admitted_routing_policy_ref
    }

    /// Returns the exact provider-qualified activation sealed into the authority.
    pub const fn domain_activation_attestation(&self) -> &WalletNonceDomainActivationAttestation {
        &self.domain_activation_attestation
    }

    /// Returns the stable wallet-authority resource lineage contract.
    pub const fn resource_contract_ref(&self) -> &ContentRef {
        &self.resource_contract_ref
    }

    /// Returns the exact public wallet-read physical certificate.
    pub fn read_certificate(&self) -> &HistoryObject {
        self.read_release_history.current().certificate()
    }

    /// Returns the exact public wallet-effect physical certificate.
    pub fn effect_certificate(&self) -> &HistoryObject {
        self.effect_release_history.current().certificate()
    }

    /// Returns every retained wallet-read physical release.
    pub const fn read_release_history(&self) -> &EvmPhysicalBindingReleaseHistory {
        &self.read_release_history
    }

    /// Returns every retained wallet-effect physical release.
    pub const fn effect_release_history(&self) -> &EvmPhysicalBindingReleaseHistory {
        &self.effect_release_history
    }
}

/// Proves the selected release target is the authority's current physical incarnation.
fn authority_release_is_current(
    authority: &dyn WalletNonceAuthority,
    release_target: &ContentRef,
) -> bool {
    authority
        .domain_activation_attestation()
        .initial_store_incarnation_ref
        .to_content_ref()
        .ok()
        .is_some_and(|current| &current == release_target)
}

trait WalletReadSpec: RuntimeReadCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract>;

    fn invoke<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a Self::Request,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<Self::Returned, Self::SafeFailure>>;
}

impl WalletReadSpec for ReadWalletNonceStatusCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        read_wallet_nonce_status_adapter_contract()
    }

    fn invoke<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a ReadEvmWalletNonceStatusRequest,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<WalletNonceStatus, EvmSubmissionFailure>> {
        authority.read_status(state_input_ref, request)
    }
}

/// Opaque wallet-status binding carrying the exact current state input
/// provenance selected before Runtime authorization.
#[doc(hidden)]
pub struct EvmStructuredWalletReadBinding<C> {
    source: EvmStructuredWalletBindings,
    state_input_ref: LexicalValueRef,
    _capability: PhantomData<fn() -> C>,
}

impl<C> ReadAdapterInvoker<C> for EvmStructuredWalletReadBinding<C>
where
    C: WalletReadSpec,
{
    fn invoke<'a>(
        &'a self,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<C::Returned, C::SafeFailure>> {
        C::invoke(
            self.source.authority.as_ref(),
            &self.state_input_ref,
            request,
        )
    }
}

impl<C> RuntimeReadAdapter<C> for EvmStructuredWalletReadBinding<C>
where
    C: WalletReadSpec,
{
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        <C as WalletReadSpec>::contract()
    }
}

impl<C> RuntimeReadPhysicalBinding<C> for EvmStructuredWalletReadBinding<C>
where
    C: WalletReadSpec,
{
    fn public_certificate(&self) -> &HistoryObject {
        self.source.read_release_history.current().certificate()
    }
}

impl<C> RuntimeReadPhysicalBindingSource<C> for EvmStructuredWalletBindings
where
    C: WalletReadSpec,
{
    type Binding = EvmStructuredWalletReadBinding<C>;

    fn current_binding<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        _request: &'a C::Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        let binding = self.read_selection(selection).then(|| {
            Arc::new(EvmStructuredWalletReadBinding {
                source: self.clone(),
                state_input_ref: selection.state_input_ref.clone(),
                _capability: PhantomData,
            })
        });
        Box::pin(async move { binding })
    }
}

trait WalletEffectSpec: RuntimeEffectCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract>;

    fn invoke<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a Self::Request,
    ) -> ComponentFuture<'a, mfm_capabilities::EffectContractCompletion<Self>>;
}

impl WalletEffectSpec for ReserveWalletNonceCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        reserve_wallet_nonce_adapter_contract()
    }

    fn invoke<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a ReserveEvmNonceRequest,
    ) -> ComponentFuture<'a, mfm_capabilities::EffectContractCompletion<ReserveWalletNonceCapability>>
    {
        authority.reserve(state_input_ref, request)
    }
}

impl WalletEffectSpec for ActivateWalletCandidateCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        activate_wallet_candidate_adapter_contract()
    }

    fn invoke<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a ActivateEvmCandidateRequest,
    ) -> ComponentFuture<
        'a,
        mfm_capabilities::EffectContractCompletion<ActivateWalletCandidateCapability>,
    > {
        authority.activate_candidate(state_input_ref, request)
    }
}

impl WalletEffectSpec for CompleteWalletNonceCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        complete_wallet_nonce_adapter_contract()
    }

    fn invoke<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a CompleteEvmNonceRequest,
    ) -> ComponentFuture<
        'a,
        mfm_capabilities::EffectContractCompletion<CompleteWalletNonceCapability>,
    > {
        authority.complete(state_input_ref, request)
    }
}

/// Opaque target-bound wallet mutation adapter carrying exact state-input
/// provenance and no transferable database authority.
#[doc(hidden)]
pub struct EvmStructuredWalletEffectBinding<C> {
    source: EvmStructuredWalletBindings,
    state_input_ref: LexicalValueRef,
    _capability: PhantomData<fn() -> C>,
}

impl<C> EffectAdapterInvoker<C> for EvmStructuredWalletEffectBinding<C>
where
    C: WalletEffectSpec,
{
    fn invoke<'a>(
        &'a self,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, mfm_capabilities::EffectContractCompletion<C>> {
        C::invoke(
            self.source.authority.as_ref(),
            &self.state_input_ref,
            request,
        )
    }
}

impl<C> RuntimeEffectAdapter<C> for EvmStructuredWalletEffectBinding<C>
where
    C: WalletEffectSpec,
{
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        <C as WalletEffectSpec>::contract()
    }
}

impl<C> RuntimeEffectPhysicalBinding<C> for EvmStructuredWalletEffectBinding<C>
where
    C: WalletEffectSpec,
{
    fn public_certificate(&self) -> &HistoryObject {
        self.source.effect_release_history.current().certificate()
    }

    fn supersession_head<'a>(
        &'a self,
        evidence: &'a <C::Refresh as EffectRefreshMode>::Evidence,
    ) -> ComponentFuture<'a, Option<HistoryObject>> {
        let evidence = serde_json::to_value(evidence).ok();
        let authority = Arc::clone(&self.source.authority);
        Box::pin(async move {
            let evidence = evidence.and_then(|value| serde_json::from_value(value).ok());
            match evidence {
                Some(evidence) => authority.supersession_head(&evidence).await,
                None => None,
            }
        })
    }
}

impl<C> RuntimeEffectPhysicalBindingSource<C> for EvmStructuredWalletBindings
where
    C: WalletEffectSpec,
{
    type Binding = EvmStructuredWalletEffectBinding<C>;

    fn current_binding<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        _request: &'a C::Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        let binding = self.effect_selection(selection).then(|| {
            Arc::new(EvmStructuredWalletEffectBinding {
                source: self.clone(),
                state_input_ref: selection.state_input_ref.clone(),
                _capability: PhantomData,
            })
        });
        Box::pin(async move { binding })
    }
}

impl BoundedComponentInvoker<WalletNonceAuthorityResource> for EvmStructuredWalletBindings {
    fn invoke<'a>(&'a self, _request: &'a ()) -> ComponentFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// Registers the status/reserve/activate/complete target sources and the sole
/// wallet resource implementation, returning every exact physical purpose tuple.
pub fn register_evm_wallet_authority_bindings(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    bindings: Arc<EvmStructuredWalletBindings>,
) -> mfm_certify::Result<Vec<EvmPhysicalBindingPurpose>> {
    let purposes = vec![
        register_read_adapter::<ReadWalletNonceStatusCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/wallet-status-adapter",
        )?,
        register_effect_adapter::<ReserveWalletNonceCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/wallet-reserve-adapter",
        )?,
        register_effect_adapter::<ActivateWalletCandidateCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/wallet-activate-adapter",
        )?,
        register_effect_adapter::<CompleteWalletNonceCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/wallet-complete-adapter",
        )?,
    ];
    let resource_contract = WalletNonceAuthorityResource::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(certification_error)?;
    registry.register_resource_authority::<WalletNonceAuthorityResource, _>(
        implementation_descriptor(
            StructuredComponentKind::Resource,
            resource_contract,
            stable("mfm.evm-live.implementation/wallet-authority-resource")?,
            qualification,
        ),
        bindings,
    )?;
    Ok(purposes)
}

fn register_read_adapter<C>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    bindings: Arc<EvmStructuredWalletBindings>,
    implementation_id: &str,
) -> mfm_certify::Result<EvmPhysicalBindingPurpose>
where
    C: WalletReadSpec,
{
    let adapter_contract_ref = <C as WalletReadSpec>::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(certification_error)?;
    let capability_contract_ref = <C as RuntimeReadCapability>::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(certification_error)?;
    let descriptor = implementation_descriptor(
        StructuredComponentKind::Adapter,
        adapter_contract_ref.clone(),
        stable(implementation_id)?,
        qualification,
    );
    let adapter_implementation_ref = descriptor.content_ref().map_err(certification_error)?;
    let purpose = EvmPhysicalBindingPurpose::new(
        AccessKind::Read,
        capability_contract_ref,
        adapter_contract_ref,
        adapter_implementation_ref,
        None,
        bindings.read_release_history.clone(),
    )
    .map_err(certification_error)?;
    registry.register_read_adapter::<C, _>(descriptor, bindings)?;
    Ok(purpose)
}

fn register_effect_adapter<C>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    bindings: Arc<EvmStructuredWalletBindings>,
    implementation_id: &str,
) -> mfm_certify::Result<EvmPhysicalBindingPurpose>
where
    C: WalletEffectSpec,
{
    let adapter_contract_ref = <C as WalletEffectSpec>::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(certification_error)?;
    let capability_contract_ref = <C as RuntimeEffectCapability>::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(certification_error)?;
    let descriptor = implementation_descriptor(
        StructuredComponentKind::Adapter,
        adapter_contract_ref.clone(),
        stable(implementation_id)?,
        qualification,
    );
    let adapter_implementation_ref = descriptor.content_ref().map_err(certification_error)?;
    let purpose = EvmPhysicalBindingPurpose::new(
        AccessKind::Effect,
        capability_contract_ref,
        adapter_contract_ref,
        adapter_implementation_ref,
        Some(bindings.resource_contract_ref.clone()),
        bindings.effect_release_history.clone(),
    )
    .map_err(certification_error)?;
    registry.register_effect_adapter::<C, _>(descriptor, bindings)?;
    Ok(purpose)
}

fn implementation_descriptor(
    component_kind: StructuredComponentKind,
    semantic_contract_ref: ContentRef,
    implementation_id: StableId,
    qualification: &EvmSubmissionProcessQualification,
) -> SecretFreeImplementationDescriptor {
    SecretFreeImplementationDescriptor {
        component_kind,
        semantic_contract_ref,
        implementation_id,
        executable_identity_ref: qualification.executable_identity_ref().clone(),
        qualification_artifact_ref: qualification.qualification_artifact_ref().clone(),
    }
}

fn stable(value: &str) -> mfm_certify::Result<StableId> {
    StableId::new(value).map_err(certification_error)
}

fn certification_error(error: impl std::fmt::Display) -> mfm_certify::CertifyError {
    mfm_certify::CertifyError::Certification(error.to_string())
}
