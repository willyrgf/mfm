//! Direct structured-Runtime EVM transport and signer bindings.

use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use mfm_capabilities::{
    AccessFaultCode, BoundedComponentInvoker, ComponentFuture, EffectAdapterCompletion,
    EffectAdapterInvoker, EffectRefreshMode, ReadAdapterCompletion, ReadAdapterInvoker,
};
use mfm_certify::structured::{
    PhysicalBindingSelection, ProgramRegistryBuilder, RuntimeEffectPhysicalBinding,
    RuntimeEffectPhysicalBindingSource, RuntimeReadPhysicalBinding,
    RuntimeReadPhysicalBindingSource,
};
use mfm_evm::{
    canonical_wallet_reference, evm_broadcast_adapter_contract,
    evm_finalized_head_adapter_contract, evm_inclusion_block_adapter_contract,
    evm_pending_nonce_adapter_contract, evm_receipt_lookup_adapter_contract,
    evm_signer_attestation_adapter_contract, evm_transaction_lookup_adapter_contract,
    sign_eip1559_guarded, signing_failure_is_integrity, AccountAddress,
    AttestCandidateIdentityCapability, AttestCandidateIdentityRequest, AttestedWalletCandidate,
    BroadcastExactCandidateCapability, BroadcastExactCandidateRequest, BroadcastLineageHead,
    EvmBroadcastResource, EvmChainInstanceBinding, EvmFinalizedHeadCapability,
    EvmFinalizedHeadObservation, EvmFinalizedHeadRequest, EvmInclusionBlockCapability,
    EvmInclusionBlockObservation, EvmInclusionBlockRequest, EvmPendingNonceCapability,
    EvmPendingNonceRequest, EvmReceiptLookupCapability, EvmReceiptLookupObservation,
    EvmReceiptLookupRequest, EvmRoutingGenerationRef, EvmSubmissionFailure,
    EvmSubmissionProcessQualification, EvmTransactionLookupCapability,
    EvmTransactionLookupObservation, EvmTransactionLookupRequest, EvmWalletReceiptStatus,
    EvmWalletReference, ObservedPendingNonceFloor, SubmittedCandidateProof, TransactionNonce,
    UnsignedWalletCandidate,
};
use mfm_ids::{ContentRef, StableId};
use mfm_journal::structured::{AccessKind, HistoryObject};
use mfm_program::structured::{
    RuntimeEffectAdapter, RuntimeEffectCapability, RuntimeReadAdapter, RuntimeReadCapability,
    RuntimeResourceAuthority,
};
use mfm_signing::QualifiedReadSigningProvider;
use mfm_spec::structured::{
    SecretFreeImplementationDescriptor, StructuredComponentKind, StructuredLiveComponentContract,
};

use crate::transport::{EvmJsonRpcTransport, WalletBroadcastResponse, WalletRpcFailure};
use crate::{EvmPhysicalBindingPurpose, EvmPhysicalBindingReleaseHistory};

/// Redaction-safe construction failure for direct structured EVM bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmStructuredLiveBindingError {
    /// Public route, signer, certificate, or lineage material was inconsistent.
    #[error("structured EVM live binding is invalid")]
    InvalidContract,
}

/// One exact route/signer target set used by the direct structured Runtime.
#[derive(Clone)]
pub struct EvmStructuredLiveBindings {
    transport: Arc<EvmJsonRpcTransport>,
    chain_instance: EvmChainInstanceBinding,
    route_generation_ref: EvmWalletReference,
    admitted_routing_policy_ref: ContentRef,
    semantic_signer_id: StableId,
    expected_sender: Address,
    signer: QualifiedReadSigningProvider,
    rpc_release_history: EvmPhysicalBindingReleaseHistory,
    signer_release_history: EvmPhysicalBindingReleaseHistory,
    broadcast_release_history: EvmPhysicalBindingReleaseHistory,
    broadcast_lineage_head: BroadcastLineageHead,
    broadcast_lineage_head_object: HistoryObject,
    integrity_fault: AccessFaultCode,
    entry_unknown_fault: AccessFaultCode,
}

impl EvmStructuredLiveBindings {
    /// Qualifies one immutable route, Read-qualified deterministic signer, and complete
    /// retained public release histories without performing provider or signer IO.
    ///
    /// Raw guarded providers are not accepted; the signer must already have passed the sole
    /// process-local Read-attestation qualification path.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transport: Arc<EvmJsonRpcTransport>,
        chain_instance: EvmChainInstanceBinding,
        route_generation_ref: EvmWalletReference,
        admitted_routing_policy_ref: ContentRef,
        semantic_signer_id: StableId,
        expected_sender: Address,
        signer: QualifiedReadSigningProvider,
        rpc_release_history: EvmPhysicalBindingReleaseHistory,
        signer_release_history: EvmPhysicalBindingReleaseHistory,
        broadcast_release_history: EvmPhysicalBindingReleaseHistory,
        broadcast_lineage_head: BroadcastLineageHead,
        broadcast_lineage_head_object: HistoryObject,
    ) -> Result<Self, EvmStructuredLiveBindingError> {
        let route_generation = route_generation_ref
            .to_content_ref()
            .ok()
            .and_then(|reference| EvmRoutingGenerationRef::from_content_ref(reference).ok());
        let route_target_ref = route_generation_ref.to_content_ref().ok();
        let signer_target_ref = signer.binding().durable_generation_ref();
        let broadcast_release = broadcast_release_history.current();
        if !signer.is_read_attestation_eligible()
            || chain_instance.validate().is_err()
            || expected_sender.is_zero()
            || route_generation
                .as_ref()
                .is_none_or(|generation| !transport.qualifies_route(generation, &chain_instance))
            || rpc_release_history.current().admitted_routing_policy_ref()
                != &admitted_routing_policy_ref
            || signer_release_history
                .current()
                .admitted_routing_policy_ref()
                != &admitted_routing_policy_ref
            || broadcast_release_history
                .current()
                .admitted_routing_policy_ref()
                != &admitted_routing_policy_ref
            || route_target_ref.as_ref()
                != Some(rpc_release_history.current().physical_target_ref())
            || signer_release_history.current().physical_target_ref() != signer_target_ref
            || broadcast_release.physical_target_ref() != &broadcast_lineage_head_object.content_ref
            || broadcast_release.predecessor_binding_ref().is_some()
                && broadcast_release.activation_lineage_head_ref()
                    != Some(&broadcast_lineage_head_object.content_ref)
            || broadcast_lineage_head.validate().is_err()
            || broadcast_lineage_head
                .public_lineage_head_ref
                .to_content_ref()
                .ok()
                != Some(broadcast_lineage_head_object.content_ref.clone())
            || broadcast_lineage_head_object.validate().is_err()
            || signer.binding().expected_public_identity().account_id()
                != Some(format!("{expected_sender:#x}").as_str())
        {
            return Err(EvmStructuredLiveBindingError::InvalidContract);
        }
        let integrity_fault = AccessFaultCode::new(
            StableId::new("mfm.evm-live/structured-adapter-integrity-fault")
                .map_err(|_| EvmStructuredLiveBindingError::InvalidContract)?,
        );
        let entry_unknown_fault = AccessFaultCode::new(
            StableId::new("mfm.evm-live/broadcast-entry-unknown")
                .map_err(|_| EvmStructuredLiveBindingError::InvalidContract)?,
        );
        Ok(Self {
            transport,
            chain_instance,
            route_generation_ref,
            admitted_routing_policy_ref,
            semantic_signer_id,
            expected_sender,
            signer,
            rpc_release_history,
            signer_release_history,
            broadcast_release_history,
            broadcast_lineage_head,
            broadcast_lineage_head_object,
            integrity_fault,
            entry_unknown_fault,
        })
    }

    fn route_ref(&self) -> Option<ContentRef> {
        self.route_generation_ref.to_content_ref().ok()
    }

    /// Returns the immutable admitted routing policy selected by these bindings.
    pub const fn admitted_routing_policy_ref(&self) -> &ContentRef {
        &self.admitted_routing_policy_ref
    }

    /// Returns the exact qualified physical-chain binding.
    pub const fn chain_instance(&self) -> &EvmChainInstanceBinding {
        &self.chain_instance
    }

    /// Returns the exact route generation used by every wallet RPC.
    pub const fn route_generation_ref(&self) -> &EvmWalletReference {
        &self.route_generation_ref
    }

    /// Returns the complete secret-free catalog bound by the transport.
    pub fn routing_catalog_descriptor(&self) -> &mfm_evm::EvmRoutingCatalogDescriptor {
        self.transport.routing_catalog_descriptor()
    }

    /// Returns the exact public RPC physical certificate.
    pub fn rpc_certificate(&self) -> &HistoryObject {
        self.rpc_release_history.current().certificate()
    }

    /// Returns the exact public signer physical certificate.
    pub fn signer_certificate(&self) -> &HistoryObject {
        self.signer_release_history.current().certificate()
    }

    /// Returns the exact public broadcast physical certificate.
    pub fn broadcast_certificate(&self) -> &HistoryObject {
        self.broadcast_release_history.current().certificate()
    }

    /// Returns every retained RPC physical release.
    pub const fn rpc_release_history(&self) -> &EvmPhysicalBindingReleaseHistory {
        &self.rpc_release_history
    }

    /// Returns every retained signer physical release.
    pub const fn signer_release_history(&self) -> &EvmPhysicalBindingReleaseHistory {
        &self.signer_release_history
    }

    /// Returns every retained broadcast physical release.
    pub const fn broadcast_release_history(&self) -> &EvmPhysicalBindingReleaseHistory {
        &self.broadcast_release_history
    }

    /// Returns the current public broadcast-lineage head object.
    pub const fn broadcast_lineage_head_object(&self) -> &HistoryObject {
        &self.broadcast_lineage_head_object
    }

    fn common_selection(&self, selection: PhysicalBindingSelection<'_>) -> bool {
        selection.admitted_routing_policy_ref == &self.admitted_routing_policy_ref
    }

    fn broadcast_selection(&self, selection: PhysicalBindingSelection<'_>) -> bool {
        self.common_selection(selection)
            && selection
                .minimum_lineage_head_ref
                .is_none_or(|minimum| minimum == &self.broadcast_lineage_head_object.content_ref)
    }

    fn valid_signer_candidate(&self, candidate: &UnsignedWalletCandidate) -> bool {
        candidate.transaction_intent.chain_instance() == &self.chain_instance
            && candidate.transaction_intent.semantic_signer_id() == self.semantic_signer_id.as_str()
            && candidate.transaction_intent.nonce_domain().sender()
                == format!("{:#x}", self.expected_sender)
            && candidate.semantic_reservation_key.validate().is_ok()
            && candidate
                .transaction_intent
                .unsigned_candidate(candidate.nonce, &candidate.fee)
                .is_ok_and(|envelope| {
                    candidate.unsigned_candidate_digest
                        == format!("{:#x}", envelope.signing_digest())
                })
    }

    async fn sign_candidate(
        &self,
        candidate: &UnsignedWalletCandidate,
    ) -> Result<
        (
            mfm_evm::TransientSignedEip1559Envelope,
            EvmWalletReference,
            EvmWalletReference,
        ),
        LiveInvocationFailure,
    > {
        if !self.valid_signer_candidate(candidate) {
            return Err(LiveInvocationFailure::Integrity);
        }
        let envelope = candidate
            .transaction_intent
            .unsigned_candidate(candidate.nonce, &candidate.fee)
            .map_err(|_| LiveInvocationFailure::Integrity)?;
        let generation_ref = self.signer.binding().durable_generation_ref().clone();
        let descriptor = self
            .signer
            .binding()
            .public_descriptor()
            .map_err(|_| LiveInvocationFailure::Integrity)?;
        let expected_sender = AccountAddress::new(self.expected_sender)
            .map_err(|_| LiveInvocationFailure::Integrity)?;
        let signed = match sign_eip1559_guarded(
            &envelope,
            self.signer.binding().signer_ref().clone(),
            expected_sender,
            &generation_ref,
            &self.signer,
        )
        .await
        {
            Ok(signed) => signed,
            Err(error) if signing_failure_is_integrity(&error) => {
                // Integrity/contract violations are invalid evidence, never
                // ordinary signer unavailability.
                return Err(LiveInvocationFailure::Integrity);
            }
            Err(_) => {
                return Err(LiveInvocationFailure::Safe(
                    EvmSubmissionFailure::SignerUnavailable,
                ));
            }
        };
        Ok((
            signed,
            EvmWalletReference::from_content_ref(generation_ref),
            EvmWalletReference::from_content_ref(descriptor.reference().clone()),
        ))
    }

    async fn attest_candidate(
        &self,
        request: &AttestCandidateIdentityRequest,
    ) -> Result<AttestedWalletCandidate, LiveInvocationFailure> {
        let (signed, _generation_ref, signer_attestation_ref) =
            self.sign_candidate(&request.candidate).await?;
        let transaction_hash = signed.transaction_hash();
        drop(signed);
        Ok(AttestedWalletCandidate {
            semantic_reservation_key: request.candidate.semantic_reservation_key.clone(),
            candidate_ordinal: request.candidate.candidate_ordinal,
            candidate_descriptor_ref: canonical_wallet_reference(&request.candidate)
                .map_err(|_| LiveInvocationFailure::Integrity)?,
            unsigned_candidate_digest: request.candidate.unsigned_candidate_digest.clone(),
            transaction_hash: format!("{transaction_hash:#x}"),
            semantic_signer_id: self.semantic_signer_id.as_str().to_owned(),
            signing_profile_contract_ref: request
                .candidate
                .transaction_intent
                .signing_profile_contract_ref()
                .clone(),
            signer_attestation_ref,
        })
    }

    async fn broadcast(
        &self,
        request: &BroadcastExactCandidateRequest,
    ) -> mfm_capabilities::EffectContractCompletion<BroadcastExactCandidateCapability> {
        if request.route_generation_ref != self.route_generation_ref
            || request
                .active_candidate
                .attested_candidate
                .semantic_reservation_key
                != request.unsigned_candidate.semantic_reservation_key
            || request
                .active_candidate
                .attested_candidate
                .candidate_ordinal
                != request.unsigned_candidate.candidate_ordinal
            || request
                .active_candidate
                .attested_candidate
                .unsigned_candidate_digest
                != request.unsigned_candidate.unsigned_candidate_digest
        {
            return EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
        }
        let (signed, signer_generation_ref, signing_contract_ref) =
            match self.sign_candidate(&request.unsigned_candidate).await {
                Ok(value) => value,
                Err(LiveInvocationFailure::Safe(failure)) => {
                    return EffectAdapterCompletion::SafeFailure(failure);
                }
                Err(LiveInvocationFailure::Integrity) => {
                    return EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                }
            };
        let transaction_hash = signed.transaction_hash();
        if format!("{transaction_hash:#x}")
            != request.active_candidate.attested_candidate.transaction_hash
        {
            return EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
        }
        let Some(route) = self.route_ref() else {
            return EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
        };
        match self
            .transport
            .send_raw_transaction(&route, &self.chain_instance, signed)
            .await
        {
            Ok(WalletBroadcastResponse::Accepted(acknowledged))
                if acknowledged == transaction_hash =>
            {
                self.submitted_proof(
                    request,
                    transaction_hash,
                    signer_generation_ref,
                    signing_contract_ref,
                )
            }
            Ok(WalletBroadcastResponse::AlreadyKnown) => self.submitted_proof(
                request,
                transaction_hash,
                signer_generation_ref,
                signing_contract_ref,
            ),
            Ok(WalletBroadcastResponse::Accepted(_)) => {
                EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone())
            }
            Err(WalletRpcFailure::GenerationFenced) => {
                EffectAdapterCompletion::SupersededBeforeEntry(self.broadcast_lineage_head.clone())
            }
            Err(WalletRpcFailure::AccessCancelled | WalletRpcFailure::UnavailableBeforeEntry) => {
                EffectAdapterCompletion::SafeFailure(EvmSubmissionFailure::TransportUnavailable)
            }
            Err(WalletRpcFailure::DestinationRejected) => {
                EffectAdapterCompletion::SafeFailure(EvmSubmissionFailure::DestinationRejected)
            }
            Err(WalletRpcFailure::ResponseLost | WalletRpcFailure::InvalidResponse) => {
                EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone())
            }
        }
    }

    fn submitted_proof(
        &self,
        request: &BroadcastExactCandidateRequest,
        transaction_hash: B256,
        signer_generation_ref: EvmWalletReference,
        signing_contract_ref: EvmWalletReference,
    ) -> mfm_capabilities::EffectContractCompletion<BroadcastExactCandidateCapability> {
        EffectAdapterCompletion::Returned(SubmittedCandidateProof {
            candidate_ordinal: request.unsigned_candidate.candidate_ordinal,
            unsigned_candidate_digest: request.unsigned_candidate.unsigned_candidate_digest.clone(),
            transaction_hash: format!("{transaction_hash:#x}"),
            semantic_signer_id: self.semantic_signer_id.as_str().to_owned(),
            signer_generation_ref,
            signing_contract_ref,
            submission_contract_ref: request
                .unsigned_candidate
                .transaction_intent
                .submission_contract_ref()
                .clone(),
        })
    }
}

enum LiveInvocationFailure {
    Safe(EvmSubmissionFailure),
    Integrity,
}

trait StructuredReadSpec: RuntimeReadCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract>;

    fn eligible(
        source: &EvmStructuredLiveBindings,
        selection: PhysicalBindingSelection<'_>,
        request: &Self::Request,
    ) -> bool;

    fn release_history(source: &EvmStructuredLiveBindings) -> &EvmPhysicalBindingReleaseHistory;

    fn invoke<'a>(
        source: &'a EvmStructuredLiveBindings,
        request: &'a Self::Request,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<Self::Returned, Self::SafeFailure>>;
}

/// Opaque target-bound Read adapter returned by
/// [`EvmStructuredLiveBindings`].
#[doc(hidden)]
pub struct EvmStructuredReadBinding<C> {
    source: EvmStructuredLiveBindings,
    _capability: PhantomData<fn() -> C>,
}

impl<C> ReadAdapterInvoker<C> for EvmStructuredReadBinding<C>
where
    C: StructuredReadSpec,
{
    fn invoke<'a>(
        &'a self,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<C::Returned, C::SafeFailure>> {
        C::invoke(&self.source, request)
    }
}

impl<C> RuntimeReadAdapter<C> for EvmStructuredReadBinding<C>
where
    C: StructuredReadSpec,
{
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        <C as StructuredReadSpec>::contract()
    }
}

impl<C> RuntimeReadPhysicalBinding<C> for EvmStructuredReadBinding<C>
where
    C: StructuredReadSpec,
{
    fn public_certificate(&self) -> &HistoryObject {
        C::release_history(&self.source).current().certificate()
    }
}

impl<C> RuntimeReadPhysicalBindingSource<C> for EvmStructuredLiveBindings
where
    C: StructuredReadSpec,
{
    type Binding = EvmStructuredReadBinding<C>;

    fn current_binding<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        let binding = C::eligible(self, selection, request).then(|| {
            Arc::new(EvmStructuredReadBinding {
                source: self.clone(),
                _capability: PhantomData,
            })
        });
        Box::pin(async move { binding })
    }
}

impl StructuredReadSpec for EvmPendingNonceCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        evm_pending_nonce_adapter_contract()
    }

    fn eligible(
        source: &EvmStructuredLiveBindings,
        selection: PhysicalBindingSelection<'_>,
        request: &EvmPendingNonceRequest,
    ) -> bool {
        source.common_selection(selection)
            && source.route_ref() == request.route_generation_ref.to_content_ref().ok()
            && request.nonce_domain.sender() == format!("{:#x}", source.expected_sender)
    }

    fn release_history(source: &EvmStructuredLiveBindings) -> &EvmPhysicalBindingReleaseHistory {
        &source.rpc_release_history
    }

    fn invoke<'a>(
        source: &'a EvmStructuredLiveBindings,
        request: &'a EvmPendingNonceRequest,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<ObservedPendingNonceFloor, EvmSubmissionFailure>>
    {
        Box::pin(async move {
            let Some(route) = source.route_ref() else {
                return ReadAdapterCompletion::IntegrityFault(source.integrity_fault.clone());
            };
            match source
                .transport
                .pending_nonce(&route, &source.chain_instance, source.expected_sender)
                .await
            {
                Ok(pending) => match u64::try_from(pending)
                    .ok()
                    .and_then(|value| TransactionNonce::new(value).ok())
                {
                    Some(pending_nonce) => {
                        ReadAdapterCompletion::Returned(ObservedPendingNonceFloor {
                            nonce_domain: request.nonce_domain.clone(),
                            route_generation_ref: request.route_generation_ref.clone(),
                            pending_nonce,
                        })
                    }
                    None => ReadAdapterCompletion::IntegrityFault(source.integrity_fault.clone()),
                },
                Err(failure) => source.read_failure(failure),
            }
        })
    }
}

impl StructuredReadSpec for AttestCandidateIdentityCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        evm_signer_attestation_adapter_contract()
    }

    fn eligible(
        source: &EvmStructuredLiveBindings,
        selection: PhysicalBindingSelection<'_>,
        request: &AttestCandidateIdentityRequest,
    ) -> bool {
        source.common_selection(selection) && source.valid_signer_candidate(&request.candidate)
    }

    fn release_history(source: &EvmStructuredLiveBindings) -> &EvmPhysicalBindingReleaseHistory {
        &source.signer_release_history
    }

    fn invoke<'a>(
        source: &'a EvmStructuredLiveBindings,
        request: &'a AttestCandidateIdentityRequest,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<AttestedWalletCandidate, EvmSubmissionFailure>>
    {
        Box::pin(async move {
            match source.attest_candidate(request).await {
                Ok(attested) => ReadAdapterCompletion::Returned(attested),
                Err(LiveInvocationFailure::Safe(failure)) => {
                    ReadAdapterCompletion::SafeFailure(failure)
                }
                Err(LiveInvocationFailure::Integrity) => {
                    ReadAdapterCompletion::IntegrityFault(source.integrity_fault.clone())
                }
            }
        })
    }
}

macro_rules! route_read_spec {
    (
        $capability:ty,
        $request:ty,
        $returned:ty,
        contract = $contract:path,
        invoke = |$source:ident, $request_value:ident, $route:ident| $invoke:block
    ) => {
        impl StructuredReadSpec for $capability {
            fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
                $contract()
            }

            fn eligible(
                source: &EvmStructuredLiveBindings,
                selection: PhysicalBindingSelection<'_>,
                request: &$request,
            ) -> bool {
                source.common_selection(selection)
                    && source.route_ref() == request.route_generation_ref.to_content_ref().ok()
            }

            fn release_history(
                source: &EvmStructuredLiveBindings,
            ) -> &EvmPhysicalBindingReleaseHistory {
                &source.rpc_release_history
            }

            fn invoke<'a>(
                source: &'a EvmStructuredLiveBindings,
                request: &'a $request,
            ) -> ComponentFuture<'a, ReadAdapterCompletion<$returned, EvmSubmissionFailure>> {
                Box::pin(async move {
                    let Some(route) = source.route_ref() else {
                        return ReadAdapterCompletion::IntegrityFault(
                            source.integrity_fault.clone(),
                        );
                    };
                    let $source = source;
                    let $request_value = request;
                    let $route = route;
                    $invoke
                })
            }
        }
    };
}

route_read_spec!(
    EvmTransactionLookupCapability,
    EvmTransactionLookupRequest,
    EvmTransactionLookupObservation,
    contract = evm_transaction_lookup_adapter_contract,
    invoke = |source, request, route| {
        let transaction_hash = match B256::from_str(&request.transaction_hash) {
            Ok(value) => value,
            Err(_) => {
                return ReadAdapterCompletion::IntegrityFault(source.integrity_fault.clone());
            }
        };
        match source
            .transport
            .transaction_by_hash(&route, &source.chain_instance, transaction_hash)
            .await
        {
            Ok(None) => ReadAdapterCompletion::Returned(EvmTransactionLookupObservation::Missing),
            Ok(Some(transaction)) if transaction.transaction_hash() == request.transaction_hash => {
                let (block_number, block_hash) = transaction
                    .placement()
                    .map(|placement| {
                        (
                            Some(placement.block().number().to_owned()),
                            Some(placement.block().hash().to_owned()),
                        )
                    })
                    .unwrap_or((None, None));
                ReadAdapterCompletion::Returned(EvmTransactionLookupObservation::Found {
                    transaction_hash: request.transaction_hash.clone(),
                    block_number,
                    block_hash,
                })
            }
            Ok(Some(_)) => ReadAdapterCompletion::IntegrityFault(source.integrity_fault.clone()),
            Err(failure) => source.read_failure(failure),
        }
    }
);

route_read_spec!(
    EvmReceiptLookupCapability,
    EvmReceiptLookupRequest,
    EvmReceiptLookupObservation,
    contract = evm_receipt_lookup_adapter_contract,
    invoke = |source, request, route| {
        let transaction_hash = match B256::from_str(&request.transaction_hash) {
            Ok(value) => value,
            Err(_) => {
                return ReadAdapterCompletion::IntegrityFault(source.integrity_fault.clone());
            }
        };
        match source
            .transport
            .receipt_by_hash(&route, &source.chain_instance, transaction_hash)
            .await
        {
            Ok(None) => ReadAdapterCompletion::Returned(EvmReceiptLookupObservation::Missing),
            Ok(Some(receipt)) if receipt.transaction_hash() == request.transaction_hash => {
                ReadAdapterCompletion::Returned(EvmReceiptLookupObservation::Found {
                    transaction_hash: request.transaction_hash.clone(),
                    block_number: receipt.block().number().to_owned(),
                    block_hash: receipt.block().hash().to_owned(),
                    status: match receipt.status() {
                        EvmWalletReceiptStatus::Success => 1,
                        EvmWalletReceiptStatus::Reverted => 0,
                    },
                })
            }
            Ok(Some(_)) => ReadAdapterCompletion::IntegrityFault(source.integrity_fault.clone()),
            Err(failure) => source.read_failure(failure),
        }
    }
);

route_read_spec!(
    EvmFinalizedHeadCapability,
    EvmFinalizedHeadRequest,
    EvmFinalizedHeadObservation,
    contract = evm_finalized_head_adapter_contract,
    invoke = |source, _request, route| {
        match source
            .transport
            .finalized_head(&route, &source.chain_instance)
            .await
        {
            Ok(head) => ReadAdapterCompletion::Returned(EvmFinalizedHeadObservation {
                block_number: head.number().to_owned(),
                block_hash: head.hash().to_owned(),
            }),
            Err(failure) => source.read_failure(failure),
        }
    }
);

route_read_spec!(
    EvmInclusionBlockCapability,
    EvmInclusionBlockRequest,
    EvmInclusionBlockObservation,
    contract = evm_inclusion_block_adapter_contract,
    invoke = |source, request, route| {
        let number = match U256::from_str(&request.block_number) {
            Ok(value) if value.to_string() == request.block_number => value,
            _ => {
                return ReadAdapterCompletion::IntegrityFault(source.integrity_fault.clone());
            }
        };
        match source
            .transport
            .inclusion_block(&route, &source.chain_instance, number)
            .await
        {
            Ok(Some(block)) if block.number() == request.block_number => {
                ReadAdapterCompletion::Returned(EvmInclusionBlockObservation {
                    block_number: block.number().to_owned(),
                    block_hash: block.hash().to_owned(),
                })
            }
            Ok(Some(_)) => ReadAdapterCompletion::IntegrityFault(source.integrity_fault.clone()),
            Ok(None) => {
                ReadAdapterCompletion::SafeFailure(EvmSubmissionFailure::ProviderUnavailable)
            }
            Err(failure) => source.read_failure(failure),
        }
    }
);

impl EvmStructuredLiveBindings {
    fn read_failure<T>(
        &self,
        failure: WalletRpcFailure,
    ) -> ReadAdapterCompletion<T, EvmSubmissionFailure> {
        match failure {
            WalletRpcFailure::InvalidResponse => {
                ReadAdapterCompletion::IntegrityFault(self.integrity_fault.clone())
            }
            WalletRpcFailure::DestinationRejected | WalletRpcFailure::GenerationFenced => {
                ReadAdapterCompletion::SafeFailure(EvmSubmissionFailure::ProviderUnavailable)
            }
            WalletRpcFailure::AccessCancelled
            | WalletRpcFailure::UnavailableBeforeEntry
            | WalletRpcFailure::ResponseLost => {
                ReadAdapterCompletion::SafeFailure(EvmSubmissionFailure::TransportUnavailable)
            }
        }
    }
}

trait StructuredEffectSpec: RuntimeEffectCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract>;

    fn eligible(
        source: &EvmStructuredLiveBindings,
        selection: PhysicalBindingSelection<'_>,
        request: &Self::Request,
    ) -> bool;

    fn invoke<'a>(
        source: &'a EvmStructuredLiveBindings,
        request: &'a Self::Request,
    ) -> ComponentFuture<'a, mfm_capabilities::EffectContractCompletion<Self>>;
}

impl StructuredEffectSpec for BroadcastExactCandidateCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        evm_broadcast_adapter_contract()
    }

    fn eligible(
        source: &EvmStructuredLiveBindings,
        selection: PhysicalBindingSelection<'_>,
        request: &BroadcastExactCandidateRequest,
    ) -> bool {
        source.broadcast_selection(selection)
            && source.route_ref() == request.route_generation_ref.to_content_ref().ok()
            && source.valid_signer_candidate(&request.unsigned_candidate)
    }

    fn invoke<'a>(
        source: &'a EvmStructuredLiveBindings,
        request: &'a BroadcastExactCandidateRequest,
    ) -> ComponentFuture<
        'a,
        mfm_capabilities::EffectContractCompletion<BroadcastExactCandidateCapability>,
    > {
        Box::pin(async move { source.broadcast(request).await })
    }
}

/// Opaque target-bound Effect adapter returned by
/// [`EvmStructuredLiveBindings`].
#[doc(hidden)]
pub struct EvmStructuredEffectBinding<C> {
    source: EvmStructuredLiveBindings,
    _capability: PhantomData<fn() -> C>,
}

impl<C> EffectAdapterInvoker<C> for EvmStructuredEffectBinding<C>
where
    C: StructuredEffectSpec,
{
    fn invoke<'a>(
        &'a self,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, mfm_capabilities::EffectContractCompletion<C>> {
        C::invoke(&self.source, request)
    }
}

impl<C> RuntimeEffectAdapter<C> for EvmStructuredEffectBinding<C>
where
    C: StructuredEffectSpec,
{
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        <C as StructuredEffectSpec>::contract()
    }
}

impl<C> RuntimeEffectPhysicalBinding<C> for EvmStructuredEffectBinding<C>
where
    C: StructuredEffectSpec,
{
    fn public_certificate(&self) -> &HistoryObject {
        self.source
            .broadcast_release_history
            .current()
            .certificate()
    }

    fn supersession_head<'a>(
        &'a self,
        evidence: &'a <C::Refresh as EffectRefreshMode>::Evidence,
    ) -> ComponentFuture<'a, Option<HistoryObject>> {
        let evidence = serde_json::to_value(evidence).ok();
        let expected = serde_json::to_value(&self.source.broadcast_lineage_head).ok();
        let head =
            (evidence == expected).then(|| self.source.broadcast_lineage_head_object.clone());
        Box::pin(async move { head })
    }
}

impl<C> RuntimeEffectPhysicalBindingSource<C> for EvmStructuredLiveBindings
where
    C: StructuredEffectSpec,
{
    type Binding = EvmStructuredEffectBinding<C>;

    fn current_binding<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        let binding = C::eligible(self, selection, request).then(|| {
            Arc::new(EvmStructuredEffectBinding {
                source: self.clone(),
                _capability: PhantomData,
            })
        });
        Box::pin(async move { binding })
    }
}

impl BoundedComponentInvoker<EvmBroadcastResource> for EvmStructuredLiveBindings {
    fn invoke<'a>(&'a self, _request: &'a ()) -> ComponentFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// Registers all transport/signer adapters plus the signer and broadcast
/// resource implementations required by the structured EVM expansion.
///
/// Returns the exact physical purpose tuple emitted by every adapter registration.
pub fn register_evm_live_submission_bindings(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    bindings: Arc<EvmStructuredLiveBindings>,
) -> mfm_certify::Result<Vec<EvmPhysicalBindingPurpose>> {
    let purposes = vec![
        register_read_adapter::<EvmPendingNonceCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/pending-nonce-adapter",
        )?,
        register_read_adapter::<AttestCandidateIdentityCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/signer-attestation-adapter",
        )?,
        register_read_adapter::<EvmTransactionLookupCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/transaction-lookup-adapter",
        )?,
        register_read_adapter::<EvmReceiptLookupCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/receipt-lookup-adapter",
        )?,
        register_read_adapter::<EvmFinalizedHeadCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/finalized-head-adapter",
        )?,
        register_read_adapter::<EvmInclusionBlockCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/inclusion-block-adapter",
        )?,
        register_effect_adapter::<BroadcastExactCandidateCapability>(
            registry,
            qualification,
            Arc::clone(&bindings),
            "mfm.evm-live.implementation/broadcast-adapter",
        )?,
    ];

    registry.register_resource_authority::<EvmBroadcastResource, _>(
        implementation_descriptor(
            StructuredComponentKind::Resource,
            EvmBroadcastResource::contract()
                .and_then(|contract| contract.content_ref().map_err(Into::into))
                .map_err(certification_error)?,
            stable("mfm.evm-live.implementation/broadcast-resource")?,
            qualification,
        ),
        bindings,
    )?;
    Ok(purposes)
}

fn register_read_adapter<C>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    bindings: Arc<EvmStructuredLiveBindings>,
    implementation_id: &str,
) -> mfm_certify::Result<EvmPhysicalBindingPurpose>
where
    C: StructuredReadSpec,
{
    let adapter_contract_ref = <C as StructuredReadSpec>::contract()
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
        C::release_history(&bindings).clone(),
    )
    .map_err(certification_error)?;
    registry.register_read_adapter::<C, _>(descriptor, bindings)?;
    Ok(purpose)
}

fn register_effect_adapter<C>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    bindings: Arc<EvmStructuredLiveBindings>,
    implementation_id: &str,
) -> mfm_certify::Result<EvmPhysicalBindingPurpose>
where
    C: StructuredEffectSpec,
{
    let adapter_contract_ref = <C as StructuredEffectSpec>::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(certification_error)?;
    let capability_contract_ref = <C as RuntimeEffectCapability>::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(certification_error)?;
    let stable_resource_lineage_contract_ref = EvmBroadcastResource::contract()
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
        Some(stable_resource_lineage_contract_ref),
        bindings.broadcast_release_history.clone(),
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
