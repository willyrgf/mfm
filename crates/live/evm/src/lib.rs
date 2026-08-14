#![warn(missing_docs)]
//! Qualified EVM adapter ingress and concrete live assembly registration.
//!
//! The module owns only provider, nonce-authority, and signer entry. Domain State semantics stay
//! in `mfm-evm` and `mfm-portfolio`; each adapter receives a one-use committed Runtime call and
//! derives every request from its sealed intent and immutable binding descriptor.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_capabilities::AccessCapabilityContract;
use mfm_evm::{
    BroadcastEvidence, BroadcastIntent, EvmAccessState, EvmBalanceBindings, EvmCapability,
    EvmPureState, EvmReadEvidence, EvmReadIntent, EvmReadValue, EvmState, EvmSubmissionBindings,
    NonceReservationEvidence, NonceReservationIntent,
};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId, StableId, TenantScopeId};
use mfm_portfolio::{PortfolioContinuation, PortfolioPureState, PortfolioState};
use mfm_program::{BindingDescriptor, State};
use mfm_runtime::{
    AccessResolution, BoxFuture, CommittedCall, PreparationError, PureImplementation,
    RuntimeAssemblyBuilder, RuntimeError, UnresolvedClassification,
};
use mfm_signing::{PublicSignerKeyInstance, Signer, SigningRequest};
use mfm_storage_evm_postgres::PostgresWalletNonceStore;
use serde::{Deserialize, Serialize};

const MAX_EVM_PROVIDER_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_EVM_REQUEST_BYTES: usize = 512 * 1024;

const EVM_LIVE_ADAPTER_ID: &str = "mfm.evm.live-adapter@1";

/// Returns the stable implementation identity for the concrete EVM live adapter.
pub fn evm_live_adapter_implementation_ref() -> Result<ContentRef, EvmAdapterError> {
    content_ref("mfm.adapter-implementation", EVM_LIVE_ADAPTER_ID.as_bytes())
}

/// Derives the sole nonce-reservation effect domain for one tenant wallet domain.
pub fn wallet_nonce_effect_domain(
    tenant: &TenantScopeId,
    sender: &str,
    nonce_domain: &str,
) -> Result<StableId, EvmAdapterError> {
    if !valid_binding_text(sender, 128)
        || sender != sender.to_ascii_lowercase()
        || !valid_binding_text(nonce_domain, 256)
    {
        return Err(EvmAdapterError::InvalidTarget);
    }
    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct Domain<'a> {
        tenant: &'a TenantScopeId,
        sender: &'a str,
        nonce_domain: &'a str,
    }
    let canonical = canonical_json(&Domain {
        tenant,
        sender,
        nonce_domain,
    })?;
    StableId::new(format!(
        "mfm.evm.nonce-domain/{}",
        mfm_canonical::sha256_digest_bytes(canonical.as_bytes())
    ))
    .map_err(|_| EvmAdapterError::InvalidTarget)
}

/// Public immutable route/target identity used in a binding descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvmPhysicalTarget {
    /// Public chain id.
    pub chain_id: u64,
    /// Public endpoint identity, never a credential.
    pub endpoint_ref: ContentRef,
}

impl EvmPhysicalTarget {
    fn validate(&self) -> Result<(), EvmAdapterError> {
        (self.chain_id != 0)
            .then_some(())
            .ok_or(EvmAdapterError::InvalidTarget)
    }

    /// Returns the content identity of the complete public target, including chain identity.
    pub fn content_ref(&self) -> Result<ContentRef, EvmAdapterError> {
        self.validate()?;
        let canonical = canonical_json(self)?;
        content_ref("mfm.evm-physical-target", canonical.as_bytes())
    }
}

/// Returns the canonical public content identity for one signer key instance.
pub fn public_signer_key_instance_ref(
    identity: &PublicSignerKeyInstance,
) -> Result<ContentRef, EvmAdapterError> {
    let canonical = canonical_json(identity)?;
    content_ref("mfm.public-signer-key-instance", canonical.as_bytes())
}

fn content_ref(schema_name: &str, bytes: &[u8]) -> Result<ContentRef, EvmAdapterError> {
    let schema = SchemaId::new(
        schema_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| EvmAdapterError::InvalidTarget)?;
    ContentRef::new(schema, mfm_canonical::raw_content_digest(bytes))
        .map_err(|_| EvmAdapterError::InvalidTarget)
}

/// Redaction-safe live adapter error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmAdapterError {
    /// The public target or immutable binding identity is invalid.
    #[error("EVM adapter target is invalid")]
    InvalidTarget,
    /// The response exceeded the bounded ingress envelope.
    #[error("EVM provider response exceeds the bound")]
    Oversize,
    /// Protocol authentication or call binding failed.
    #[error("EVM provider response failed authentication")]
    Authentication,
    /// The provider outcome is unresolved and grants no generic retry authority.
    #[error("EVM provider outcome is unresolved")]
    Unresolved,
    /// The live descriptor closure cannot form one exact Runtime assembly.
    #[error("EVM live assembly is incomplete")]
    Assembly,
}

/// Bounded typed provider response after raw ingress has been discarded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmProviderResponse {
    /// Authenticated typed read value.
    Read {
        /// Call correlation echoed by the provider transport.
        call_id: StableId,
        /// Operation correlation echoed by the provider transport.
        operation: StableId,
        /// Bounded interpreted provider value.
        value: EvmReadValue,
    },
    /// Authenticated transaction hash.
    Broadcast {
        /// Call correlation echoed by the provider transport.
        call_id: StableId,
        /// Operation correlation echoed by the provider transport.
        operation: StableId,
        /// Bounded provider transaction hash.
        transaction_hash: String,
    },
    /// Reviewed provider rejection code.
    Rejected {
        /// Call correlation echoed by the provider transport.
        call_id: StableId,
        /// Operation correlation echoed by the provider transport.
        operation: StableId,
        /// Stable redacted rejection code.
        code: String,
    },
    /// The provider returned a reviewed, definite safe failure before entry.
    SafeFailure {
        /// Call correlation echoed by the provider transport.
        call_id: StableId,
        /// Operation correlation echoed by the provider transport.
        operation: StableId,
        /// Stable redacted failure code.
        code: String,
    },
    /// The provider reported that a one-entry operation may already have entered.
    PossibleEntry {
        /// Call correlation echoed by the provider transport.
        call_id: StableId,
        /// Operation correlation echoed by the provider transport.
        operation: StableId,
        /// Deterministic candidate identity associated with the possible entry.
        candidate_id: String,
    },
    /// The adapter authenticated an integrity failure that is safe to conclude as blocked.
    IntegrityBlocked {
        /// Call correlation echoed by the provider transport.
        call_id: StableId,
        /// Operation correlation echoed by the provider transport.
        operation: StableId,
        /// Stable redacted integrity code.
        code: String,
    },
}

/// Provider transport owned by one immutable qualified adapter.
pub trait EvmProvider: Send + Sync + 'static {
    /// Performs one already-authenticated request using bounded provider state.
    fn request(
        &self,
        call_id: StableId,
        operation: StableId,
        request_bytes: Vec<u8>,
    ) -> BoxFuture<Result<EvmProviderResponse, EvmAdapterError>>;
}

/// One durable nonce authority bound to a single effect descriptor.
pub trait WalletNonceAuthority: Send + Sync + 'static {
    /// Resolves a nonce reservation into exact domain evidence before provider entry.
    fn reserve(
        self: Arc<Self>,
        operation_key: StableId,
    ) -> BoxFuture<Result<NonceReservationEvidence, EvmAdapterError>>;
}

impl WalletNonceAuthority for PostgresWalletNonceStore {
    fn reserve(
        self: Arc<Self>,
        operation_key: StableId,
    ) -> BoxFuture<Result<NonceReservationEvidence, EvmAdapterError>> {
        Box::pin(async move {
            PostgresWalletNonceStore::reserve(self.as_ref(), operation_key)
                .await
                .map(|nonce| NonceReservationEvidence::Reserved { nonce })
                .map_err(|_| EvmAdapterError::Unresolved)
        })
    }
}

enum EvmAdapterHandle {
    Read(Arc<dyn EvmProvider>),
    ReserveNonce {
        sender: String,
        nonce_domain: String,
        authority: Arc<dyn WalletNonceAuthority>,
    },
    Broadcast {
        sender: String,
        nonce_domain: String,
        provider: Arc<dyn EvmProvider>,
        signer: Arc<dyn Signer>,
    },
}

/// One immutable adapter binding with one descriptor and its actual capability handle.
pub struct EvmAdapterBinding {
    descriptor: BindingDescriptor,
    target: EvmPhysicalTarget,
    handle: EvmAdapterHandle,
}

impl EvmAdapterBinding {
    /// Creates one Read binding with no signer or nonce authority.
    pub fn read(
        descriptor: BindingDescriptor,
        target: EvmPhysicalTarget,
        provider: Arc<dyn EvmProvider>,
    ) -> Result<Self, EvmAdapterError> {
        validate_descriptor_target(&descriptor, &target)?;
        if !is_read_descriptor(&descriptor) {
            return Err(EvmAdapterError::InvalidTarget);
        }
        Ok(Self {
            descriptor,
            target,
            handle: EvmAdapterHandle::Read(provider),
        })
    }

    /// Creates the durable nonce-reservation Effect binding.
    pub fn reserve_nonce(
        descriptor: BindingDescriptor,
        target: EvmPhysicalTarget,
        tenant: TenantScopeId,
        sender: String,
        nonce_domain: String,
        authority: Arc<dyn WalletNonceAuthority>,
    ) -> Result<Self, EvmAdapterError> {
        validate_descriptor_target(&descriptor, &target)?;
        if !is_reserve_nonce_descriptor(&descriptor)
            || descriptor.effect_domain()
                != Some(&wallet_nonce_effect_domain(
                    &tenant,
                    &sender,
                    &nonce_domain,
                )?)
        {
            return Err(EvmAdapterError::InvalidTarget);
        }
        Ok(Self {
            descriptor,
            target,
            handle: EvmAdapterHandle::ReserveNonce {
                sender,
                nonce_domain,
                authority,
            },
        })
    }

    /// Creates the signed broadcast Effect binding.
    pub fn broadcast(
        descriptor: BindingDescriptor,
        target: EvmPhysicalTarget,
        sender: String,
        nonce_domain: String,
        provider: Arc<dyn EvmProvider>,
        signer: Arc<dyn Signer>,
    ) -> Result<Self, EvmAdapterError> {
        validate_descriptor_target(&descriptor, &target)?;
        if !is_broadcast_descriptor(&descriptor)
            || !valid_binding_text(&sender, 128)
            || sender != sender.to_ascii_lowercase()
            || !valid_binding_text(&nonce_domain, 256)
            || descriptor.public_signer_key_instance_ref()
                != Some(&public_signer_key_instance_ref(signer.public_identity())?)
        {
            return Err(EvmAdapterError::InvalidTarget);
        }
        Ok(Self {
            descriptor,
            target,
            handle: EvmAdapterHandle::Broadcast {
                sender,
                nonce_domain,
                provider,
                signer,
            },
        })
    }

    const fn descriptor(&self) -> &BindingDescriptor {
        &self.descriptor
    }

    fn binding_matches<S: State, C: AccessCapabilityContract>(
        &self,
        call: &CommittedCall<S, C>,
    ) -> bool {
        self.descriptor == *call.binding()
            && self
                .descriptor
                .content_ref()
                .is_ok_and(|reference| reference == *call.execution_binding_ref())
            && self
                .target
                .content_ref()
                .is_ok_and(|reference| reference == *self.descriptor.physical_target_ref())
    }

    async fn read_call<S, C>(
        &self,
        call: CommittedCall<S, C>,
    ) -> Result<AccessResolution<S, C>, EvmAdapterError>
    where
        S: State,
        C: AccessCapabilityContract<Intent = EvmReadIntent, Evidence = EvmReadEvidence>,
    {
        if !self.binding_matches(&call) {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        let EvmAdapterHandle::Read(provider) = &self.handle else {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        };
        let intent = call.intent().clone();
        let (operation_name, chain_id) = intent.operation_and_chain_id();
        if chain_id != self.target.chain_id {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        let operation =
            StableId::new(operation_name).map_err(|_| EvmAdapterError::Authentication)?;
        let request = intent_request_bytes(&call)?;
        let response = match provider
            .request(call.call_id().clone(), operation.clone(), request)
            .await
        {
            Ok(response) if response_within_bound(&response) => response,
            Ok(_) => return Ok(unresolved(call, UnresolvedClassification::InvalidResponse)),
            Err(_) => {
                return Ok(unresolved(
                    call,
                    UnresolvedClassification::AcknowledgementUnknown,
                ))
            }
        };
        let evidence = match response {
            EvmProviderResponse::Read {
                call_id,
                operation: response_operation,
                value,
            } if call_id == *call.call_id() && response_operation == operation => {
                EvmReadEvidence::Returned {
                    operation: operation_name.to_owned(),
                    value,
                }
            }
            EvmProviderResponse::Rejected {
                call_id,
                operation: response_operation,
                code,
            } if call_id == *call.call_id() && response_operation == operation => {
                EvmReadEvidence::Rejected {
                    operation: operation_name.to_owned(),
                    code,
                }
            }
            EvmProviderResponse::SafeFailure {
                call_id,
                operation: response_operation,
                code,
            } if call_id == *call.call_id() && response_operation == operation => {
                EvmReadEvidence::SafeFailure {
                    operation: operation_name.to_owned(),
                    code,
                }
            }
            EvmProviderResponse::IntegrityBlocked {
                call_id,
                operation: response_operation,
                code,
            } if call_id == *call.call_id() && response_operation == operation => {
                EvmReadEvidence::IntegrityBlocked {
                    operation: operation_name.to_owned(),
                    code,
                }
            }
            _ => return Ok(unresolved(call, UnresolvedClassification::InvalidResponse)),
        };
        if C::bind_evidence(&intent, &evidence).is_err() {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        let integrity = matches!(evidence, EvmReadEvidence::IntegrityBlocked { .. });
        accept(call, evidence, integrity)
    }

    async fn reserve_nonce_call<S, C>(
        &self,
        call: CommittedCall<S, C>,
    ) -> Result<AccessResolution<S, C>, EvmAdapterError>
    where
        S: State,
        C: AccessCapabilityContract<
            Intent = NonceReservationIntent,
            Evidence = NonceReservationEvidence,
        >,
    {
        if !self.binding_matches(&call) {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        let EvmAdapterHandle::ReserveNonce {
            sender,
            nonce_domain,
            authority,
        } = &self.handle
        else {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        };
        let intent = call.intent().clone();
        if intent.validate().is_err()
            || intent.target.chain_id != self.target.chain_id
            || intent.target.sender != *sender
            || intent.target.nonce_domain != *nonce_domain
        {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        let key = nonce_operation_key(&call)?;
        let evidence = match Arc::clone(authority).reserve(key).await {
            Ok(evidence) => evidence,
            Err(_) => {
                return Ok(unresolved(
                    call,
                    UnresolvedClassification::AcknowledgementUnknown,
                ))
            }
        };
        if C::bind_evidence(&intent, &evidence).is_err() {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        let integrity = matches!(evidence, NonceReservationEvidence::IntegrityBlocked);
        accept(call, evidence, integrity)
    }

    async fn broadcast_call<S, C>(
        &self,
        call: CommittedCall<S, C>,
    ) -> Result<AccessResolution<S, C>, EvmAdapterError>
    where
        S: State,
        C: AccessCapabilityContract<Intent = BroadcastIntent, Evidence = BroadcastEvidence>,
    {
        if !self.binding_matches(&call) {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        let EvmAdapterHandle::Broadcast {
            sender,
            nonce_domain,
            provider,
            signer,
        } = &self.handle
        else {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        };
        let intent = call.intent().clone();
        if intent.validate().is_err()
            || intent.target.chain_id != self.target.chain_id
            || intent.sender != *sender
            || intent.nonce_domain != *nonce_domain
            || self.descriptor.public_signer_key_instance_ref()
                != Some(&intent.public_signer_key_instance_ref)
            || public_signer_key_instance_ref(signer.public_identity()).map_or(true, |reference| {
                reference != intent.public_signer_key_instance_ref
            })
        {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        let intent_bytes = intent_request_bytes(&call)?;
        let purpose = StableId::new("mfm.evm.broadcast-transaction@1")
            .map_err(|_| EvmAdapterError::Authentication)?;
        let signing = SigningRequest::new(
            signer.public_identity().signer_id.clone(),
            mfm_canonical::sha256_digest_bytes(&intent_bytes).to_string(),
            purpose.clone(),
        )
        .map_err(|_| EvmAdapterError::Authentication)?;
        let signed = match signer.sign(signing).await {
            Ok(value) if value.key_instance == signer.public_identity().key_instance_id => value,
            _ => {
                return Ok(unresolved(
                    call,
                    UnresolvedClassification::AcknowledgementUnknown,
                ))
            }
        };
        let request = signed_broadcast_request(&intent, &signed.signature, &signed.key_instance)?;
        let response = match provider
            .request(call.call_id().clone(), purpose.clone(), request)
            .await
        {
            Ok(response) if response_within_bound(&response) => response,
            Ok(_) => return Ok(unresolved(call, UnresolvedClassification::InvalidResponse)),
            Err(_) => {
                return Ok(unresolved(
                    call,
                    UnresolvedClassification::AcknowledgementUnknown,
                ))
            }
        };
        let evidence = match response {
            EvmProviderResponse::Broadcast {
                call_id,
                operation,
                transaction_hash,
            } if call_id == *call.call_id() && operation == purpose => {
                BroadcastEvidence::Returned {
                    candidate_id: intent.candidate_id.clone(),
                    transaction_hash,
                }
            }
            EvmProviderResponse::Rejected {
                call_id,
                operation,
                code,
            } if call_id == *call.call_id() && operation == purpose => {
                BroadcastEvidence::Rejected {
                    candidate_id: intent.candidate_id.clone(),
                    code,
                }
            }
            EvmProviderResponse::IntegrityBlocked {
                call_id, operation, ..
            } if call_id == *call.call_id() && operation == purpose => {
                BroadcastEvidence::IntegrityBlocked {
                    candidate_id: intent.candidate_id.clone(),
                }
            }
            EvmProviderResponse::PossibleEntry {
                call_id,
                operation,
                candidate_id,
            } if call_id == *call.call_id()
                && operation == purpose
                && candidate_id == intent.candidate_id =>
            {
                return Ok(unresolved(
                    call,
                    UnresolvedClassification::AcknowledgementUnknown,
                ))
            }
            _ => return Ok(unresolved(call, UnresolvedClassification::InvalidResponse)),
        };
        if C::bind_evidence(&intent, &evidence).is_err() {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        let integrity = matches!(evidence, BroadcastEvidence::IntegrityBlocked { .. });
        accept(call, evidence, integrity)
    }
}

fn unresolved<S: State, C: AccessCapabilityContract>(
    call: CommittedCall<S, C>,
    classification: UnresolvedClassification,
) -> AccessResolution<S, C> {
    AccessResolution::Unresolved(call.unresolved(classification))
}

fn accept<S: State, C: AccessCapabilityContract>(
    call: CommittedCall<S, C>,
    evidence: C::Evidence,
    integrity: bool,
) -> Result<AccessResolution<S, C>, EvmAdapterError> {
    if integrity {
        call.accept_integrity(evidence)
            .map(AccessResolution::BlockedIntegrity)
    } else {
        call.accept_evidence(evidence)
            .map(AccessResolution::Outcome)
    }
    .map_err(|_| EvmAdapterError::Authentication)
}

/// Exact live adapter and planning-binding closure for the two EVM-backed entry points.
pub struct EvmLiveAssembly {
    submission_bindings: Vec<EvmSubmissionBindings>,
    balance_bindings: Vec<EvmBalanceBindings>,
    adapters: BTreeMap<ContentRef, Arc<EvmAdapterBinding>>,
}

impl EvmLiveAssembly {
    /// Validates one complete, finite descriptor closure produced by trusted composition.
    pub fn new(
        submission_bindings: Vec<EvmSubmissionBindings>,
        balance_bindings: Vec<EvmBalanceBindings>,
        adapters: Vec<EvmAdapterBinding>,
    ) -> Result<Self, EvmAdapterError> {
        if submission_bindings.is_empty() || balance_bindings.is_empty() {
            return Err(EvmAdapterError::Assembly);
        }
        let expected = submission_bindings
            .iter()
            .flat_map(|bindings| bindings.descriptors())
            .chain(
                balance_bindings
                    .iter()
                    .flat_map(|bindings| bindings.descriptors()),
            )
            .map(|descriptor| {
                descriptor
                    .content_ref()
                    .map_err(|_| EvmAdapterError::Assembly)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut registered = BTreeMap::new();
        for adapter in adapters {
            let reference = adapter
                .descriptor()
                .content_ref()
                .map_err(|_| EvmAdapterError::Assembly)?;
            if registered.insert(reference, Arc::new(adapter)).is_some() {
                return Err(EvmAdapterError::Assembly);
            }
        }
        if registered.keys().collect::<BTreeSet<_>>() != expected.iter().collect::<BTreeSet<_>>() {
            return Err(EvmAdapterError::Assembly);
        }
        if submission_bindings
            .iter()
            .enumerate()
            .any(|(index, binding)| {
                submission_bindings[..index]
                    .iter()
                    .any(|prior| prior.target() == binding.target())
            })
            || balance_bindings.iter().enumerate().any(|(index, binding)| {
                balance_bindings[..index]
                    .iter()
                    .any(|prior| prior.chain_id == binding.chain_id)
            })
        {
            return Err(EvmAdapterError::Assembly);
        }
        for bindings in &submission_bindings {
            validate_submission_route(bindings, &registered)?;
        }
        for bindings in &balance_bindings {
            validate_balance_route(bindings, &registered)?;
        }
        Ok(Self {
            submission_bindings,
            balance_bindings,
            adapters: registered,
        })
    }

    /// Returns the exact submission and balance bindings retained for domain planning.
    pub fn planning_bindings(&self) -> (&[EvmSubmissionBindings], &[EvmBalanceBindings]) {
        (&self.submission_bindings, &self.balance_bindings)
    }

    /// Registers all domain semantic implementations and exact adapter invocations once.
    pub fn register(&self, builder: &mut RuntimeAssemblyBuilder) -> Result<(), EvmAdapterError> {
        register_semantics(builder)?;
        for bindings in &self.submission_bindings {
            let [reserve_nonce, broadcast, receipt, finalized_head, canonical_inclusion_block] =
                bindings.descriptors();
            self.register_nonce::<EvmState<0, 0>, EvmCapability<0>>(builder, reserve_nonce)?;
            self.register_broadcast::<EvmState<0, 2>, EvmCapability<1>>(builder, broadcast)?;
            self.register_read::<EvmState<0, 3>, EvmCapability<3>>(builder, receipt)?;
            self.register_read::<EvmState<0, 4>, EvmCapability<4>>(builder, finalized_head)?;
            self.register_read::<EvmState<0, 5>, EvmCapability<5>>(
                builder,
                canonical_inclusion_block,
            )?;
        }
        for bindings in &self.balance_bindings {
            let [check_chain_identity, read_initial_anchor, read_native_balance, read_token_decimals, read_token_balance, confirm_anchor] =
                bindings.descriptors();
            self.register_read::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<2>>(
                builder,
                check_chain_identity,
            )?;
            self.register_read::<EvmState<1, 1, PortfolioContinuation>, EvmCapability<6>>(
                builder,
                read_initial_anchor,
            )?;
            self.register_read::<EvmState<1, 3, PortfolioContinuation>, EvmCapability<7>>(
                builder,
                read_native_balance,
            )?;
            self.register_read::<EvmState<1, 4, PortfolioContinuation>, EvmCapability<7>>(
                builder,
                read_token_decimals,
            )?;
            self.register_read::<EvmState<1, 5, PortfolioContinuation>, EvmCapability<7>>(
                builder,
                read_token_balance,
            )?;
            self.register_read::<EvmState<1, 6, PortfolioContinuation>, EvmCapability<6>>(
                builder,
                confirm_anchor,
            )?;
        }
        Ok(())
    }

    fn adapter(
        &self,
        descriptor: &BindingDescriptor,
    ) -> Result<Arc<EvmAdapterBinding>, EvmAdapterError> {
        let reference = descriptor
            .content_ref()
            .map_err(|_| EvmAdapterError::Assembly)?;
        self.adapters
            .get(&reference)
            .filter(|adapter| adapter.descriptor() == descriptor)
            .cloned()
            .ok_or(EvmAdapterError::Assembly)
    }

    fn register_read<S, C>(
        &self,
        builder: &mut RuntimeAssemblyBuilder,
        descriptor: &BindingDescriptor,
    ) -> Result<(), EvmAdapterError>
    where
        S: State,
        C: AccessCapabilityContract<Intent = EvmReadIntent, Evidence = EvmReadEvidence>,
    {
        let adapter = self.adapter(descriptor)?;
        builder
            .register_adapter::<S, C, _>(descriptor.clone(), move |call| {
                let adapter = Arc::clone(&adapter);
                Box::pin(async move {
                    adapter
                        .read_call(call)
                        .await
                        .map_err(|_| RuntimeError::Unresolved)
                })
            })
            .map_err(|_| EvmAdapterError::Assembly)
    }

    fn register_nonce<S, C>(
        &self,
        builder: &mut RuntimeAssemblyBuilder,
        descriptor: &BindingDescriptor,
    ) -> Result<(), EvmAdapterError>
    where
        S: State,
        C: AccessCapabilityContract<
            Intent = NonceReservationIntent,
            Evidence = NonceReservationEvidence,
        >,
    {
        let adapter = self.adapter(descriptor)?;
        builder
            .register_adapter::<S, C, _>(descriptor.clone(), move |call| {
                let adapter = Arc::clone(&adapter);
                Box::pin(async move {
                    adapter
                        .reserve_nonce_call(call)
                        .await
                        .map_err(|_| RuntimeError::Unresolved)
                })
            })
            .map_err(|_| EvmAdapterError::Assembly)
    }

    fn register_broadcast<S, C>(
        &self,
        builder: &mut RuntimeAssemblyBuilder,
        descriptor: &BindingDescriptor,
    ) -> Result<(), EvmAdapterError>
    where
        S: State,
        C: AccessCapabilityContract<Intent = BroadcastIntent, Evidence = BroadcastEvidence>,
    {
        let adapter = self.adapter(descriptor)?;
        builder
            .register_adapter::<S, C, _>(descriptor.clone(), move |call| {
                let adapter = Arc::clone(&adapter);
                Box::pin(async move {
                    adapter
                        .broadcast_call(call)
                        .await
                        .map_err(|_| RuntimeError::Unresolved)
                })
            })
            .map_err(|_| EvmAdapterError::Assembly)
    }
}

fn registered_adapter<'a>(
    adapters: &'a BTreeMap<ContentRef, Arc<EvmAdapterBinding>>,
    descriptor: &BindingDescriptor,
) -> Result<&'a EvmAdapterBinding, EvmAdapterError> {
    let reference = descriptor
        .content_ref()
        .map_err(|_| EvmAdapterError::Assembly)?;
    adapters
        .get(&reference)
        .filter(|adapter| adapter.descriptor() == descriptor)
        .map(Arc::as_ref)
        .ok_or(EvmAdapterError::Assembly)
}

fn validate_submission_route(
    bindings: &EvmSubmissionBindings,
    adapters: &BTreeMap<ContentRef, Arc<EvmAdapterBinding>>,
) -> Result<(), EvmAdapterError> {
    let [reserve_nonce, broadcast_binding, receipt, finalized_head, canonical_inclusion_block] =
        bindings.descriptors();
    let reserve = registered_adapter(adapters, reserve_nonce)?;
    let EvmAdapterHandle::ReserveNonce {
        sender,
        nonce_domain,
        ..
    } = &reserve.handle
    else {
        return Err(EvmAdapterError::Assembly);
    };
    if reserve.target.chain_id != bindings.target().chain_id
        || sender != &bindings.target().sender
        || nonce_domain != &bindings.target().nonce_domain
    {
        return Err(EvmAdapterError::Assembly);
    }

    let broadcast = registered_adapter(adapters, broadcast_binding)?;
    let EvmAdapterHandle::Broadcast {
        sender,
        nonce_domain,
        ..
    } = &broadcast.handle
    else {
        return Err(EvmAdapterError::Assembly);
    };
    if broadcast.target.chain_id != bindings.target().chain_id
        || sender != &bindings.target().sender
        || nonce_domain != &bindings.target().nonce_domain
    {
        return Err(EvmAdapterError::Assembly);
    }

    for descriptor in [receipt, finalized_head, canonical_inclusion_block] {
        let adapter = registered_adapter(adapters, descriptor)?;
        if adapter.target.chain_id != bindings.target().chain_id
            || !matches!(&adapter.handle, EvmAdapterHandle::Read(_))
        {
            return Err(EvmAdapterError::Assembly);
        }
    }
    Ok(())
}

fn validate_balance_route(
    bindings: &EvmBalanceBindings,
    adapters: &BTreeMap<ContentRef, Arc<EvmAdapterBinding>>,
) -> Result<(), EvmAdapterError> {
    for descriptor in bindings.descriptors() {
        let adapter = registered_adapter(adapters, descriptor)?;
        if adapter.target.chain_id != bindings.chain_id
            || !matches!(&adapter.handle, EvmAdapterHandle::Read(_))
        {
            return Err(EvmAdapterError::Assembly);
        }
    }
    Ok(())
}

fn register_semantics(builder: &mut RuntimeAssemblyBuilder) -> Result<(), EvmAdapterError> {
    register_evm_pure::<EvmState<0, 1>>(builder)?;
    register_evm_pure::<EvmState<0, 6>>(builder)?;
    register_evm_pure::<EvmState<1, 2, PortfolioContinuation>>(builder)?;
    register_evm_pure::<EvmState<1, 7, PortfolioContinuation>>(builder)?;
    register_portfolio_pure::<PortfolioState<0>>(builder)?;
    register_portfolio_pure::<PortfolioState<1>>(builder)?;
    register_portfolio_pure::<PortfolioState<2>>(builder)?;
    register_portfolio_pure::<PortfolioState<3>>(builder)?;
    register_portfolio_pure::<PortfolioState<4>>(builder)?;

    register_access::<EvmState<0, 0>, EvmCapability<0>>(builder)?;
    register_access::<EvmState<0, 2>, EvmCapability<1>>(builder)?;
    register_access::<EvmState<0, 3>, EvmCapability<3>>(builder)?;
    register_access::<EvmState<0, 4>, EvmCapability<4>>(builder)?;
    register_access::<EvmState<0, 5>, EvmCapability<5>>(builder)?;
    register_access::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<2>>(builder)?;
    register_access::<EvmState<1, 1, PortfolioContinuation>, EvmCapability<6>>(builder)?;
    register_access::<EvmState<1, 3, PortfolioContinuation>, EvmCapability<7>>(builder)?;
    register_access::<EvmState<1, 4, PortfolioContinuation>, EvmCapability<7>>(builder)?;
    register_access::<EvmState<1, 5, PortfolioContinuation>, EvmCapability<7>>(builder)?;
    register_access::<EvmState<1, 6, PortfolioContinuation>, EvmCapability<6>>(builder)
}

fn register_evm_pure<S: EvmPureState>(
    builder: &mut RuntimeAssemblyBuilder,
) -> Result<(), EvmAdapterError> {
    builder
        .register_pure::<S>(PureImplementation::new(S::evaluate))
        .map_err(|_| EvmAdapterError::Assembly)
}

fn register_portfolio_pure<S: PortfolioPureState>(
    builder: &mut RuntimeAssemblyBuilder,
) -> Result<(), EvmAdapterError> {
    builder
        .register_pure::<S>(PureImplementation::new(S::evaluate))
        .map_err(|_| EvmAdapterError::Assembly)
}

fn register_access<S, C>(builder: &mut RuntimeAssemblyBuilder) -> Result<(), EvmAdapterError>
where
    S: EvmAccessState<C>,
    C: AccessCapabilityContract,
{
    builder
        .register_access::<S, C>(mfm_runtime::AccessImplementation::new(
            |input| S::prepare(input).map_err(|_| PreparationError),
            S::interpret,
        ))
        .map_err(|_| EvmAdapterError::Assembly)
}
fn validate_descriptor_target(
    descriptor: &BindingDescriptor,
    target: &EvmPhysicalTarget,
) -> Result<(), EvmAdapterError> {
    if descriptor.adapter_implementation_ref() != Some(&evm_live_adapter_implementation_ref()?)
        || descriptor.capability_contract_ref().is_none()
        || descriptor.physical_target_ref() != &target.content_ref()?
    {
        return Err(EvmAdapterError::InvalidTarget);
    }
    Ok(())
}

fn descriptor_has_role<S, C>(descriptor: &BindingDescriptor) -> bool
where
    S: State,
    C: AccessCapabilityContract,
{
    let Ok(state) = mfm_program::state_implementation_ref::<S>() else {
        return false;
    };
    let Ok(capability) = mfm_program::capability_contract_ref::<C>() else {
        return false;
    };
    let Ok(adapter) = evm_live_adapter_implementation_ref() else {
        return false;
    };
    descriptor.state_implementation_ref() == &state
        && descriptor.capability_contract_ref() == Some(&capability)
        && descriptor.adapter_implementation_ref() == Some(&adapter)
}

fn is_read_descriptor(descriptor: &BindingDescriptor) -> bool {
    descriptor.effect_domain().is_none()
        && descriptor.public_signer_key_instance_ref().is_none()
        && (descriptor_has_role::<EvmState<0, 3>, EvmCapability<3>>(descriptor)
            || descriptor_has_role::<EvmState<0, 4>, EvmCapability<4>>(descriptor)
            || descriptor_has_role::<EvmState<0, 5>, EvmCapability<5>>(descriptor)
            || descriptor_has_role::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<2>>(
                descriptor,
            )
            || descriptor_has_role::<EvmState<1, 1, PortfolioContinuation>, EvmCapability<6>>(
                descriptor,
            )
            || descriptor_has_role::<EvmState<1, 3, PortfolioContinuation>, EvmCapability<7>>(
                descriptor,
            )
            || descriptor_has_role::<EvmState<1, 4, PortfolioContinuation>, EvmCapability<7>>(
                descriptor,
            )
            || descriptor_has_role::<EvmState<1, 5, PortfolioContinuation>, EvmCapability<7>>(
                descriptor,
            )
            || descriptor_has_role::<EvmState<1, 6, PortfolioContinuation>, EvmCapability<6>>(
                descriptor,
            ))
}

fn is_reserve_nonce_descriptor(descriptor: &BindingDescriptor) -> bool {
    descriptor.effect_domain().is_some()
        && descriptor.public_signer_key_instance_ref().is_none()
        && descriptor_has_role::<EvmState<0, 0>, EvmCapability<0>>(descriptor)
}

fn is_broadcast_descriptor(descriptor: &BindingDescriptor) -> bool {
    descriptor.effect_domain().is_some()
        && descriptor.public_signer_key_instance_ref().is_some()
        && descriptor_has_role::<EvmState<0, 2>, EvmCapability<1>>(descriptor)
}

fn canonical_json<T: Serialize>(
    value: &T,
) -> Result<mfm_canonical::PlainCanonicalJsonBytes, EvmAdapterError> {
    let encoded = serde_json::to_string(value).map_err(|_| EvmAdapterError::Authentication)?;
    mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&encoded)
        .map_err(|_| EvmAdapterError::Authentication)
}

fn intent_request_bytes<S: State, C: AccessCapabilityContract>(
    call: &CommittedCall<S, C>,
) -> Result<Vec<u8>, EvmAdapterError> {
    let request = call
        .intent_canonical_bytes()
        .map_err(|_| EvmAdapterError::Authentication)?;
    (request.len() <= MAX_EVM_REQUEST_BYTES)
        .then_some(request)
        .ok_or(EvmAdapterError::Oversize)
}

fn nonce_operation_key<S: State, C: AccessCapabilityContract>(
    call: &CommittedCall<S, C>,
) -> Result<StableId, EvmAdapterError> {
    let bytes = intent_request_bytes(call)?;
    StableId::new(format!(
        "mfm.evm.nonce/{}",
        mfm_canonical::sha256_digest_bytes(&bytes)
    ))
    .map_err(|_| EvmAdapterError::Authentication)
}

fn signed_broadcast_request(
    intent: &BroadcastIntent,
    signature: &str,
    key_instance: &StableId,
) -> Result<Vec<u8>, EvmAdapterError> {
    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct Signed<'a> {
        intent: &'a BroadcastIntent,
        signature: &'a str,
        key_instance: &'a StableId,
    }
    if !valid_binding_text(signature, 1024) {
        return Err(EvmAdapterError::Authentication);
    }
    let request = canonical_json(&Signed {
        intent,
        signature,
        key_instance,
    })?;
    (request.as_bytes().len() <= MAX_EVM_REQUEST_BYTES)
        .then(|| request.as_bytes().to_vec())
        .ok_or(EvmAdapterError::Oversize)
}

fn response_within_bound(response: &EvmProviderResponse) -> bool {
    let size = match response {
        EvmProviderResponse::Read {
            call_id,
            operation,
            value,
        } => serde_json::to_vec(value)
            .map(|value| {
                call_id
                    .as_str()
                    .len()
                    .saturating_add(operation.as_str().len())
                    .saturating_add(value.len())
            })
            .unwrap_or(usize::MAX),
        EvmProviderResponse::Broadcast {
            call_id,
            operation,
            transaction_hash,
        } => call_id
            .as_str()
            .len()
            .saturating_add(operation.as_str().len())
            .saturating_add(transaction_hash.len()),
        EvmProviderResponse::Rejected {
            call_id,
            operation,
            code,
        }
        | EvmProviderResponse::SafeFailure {
            call_id,
            operation,
            code,
        }
        | EvmProviderResponse::IntegrityBlocked {
            call_id,
            operation,
            code,
        } => call_id
            .as_str()
            .len()
            .saturating_add(operation.as_str().len())
            .saturating_add(code.len()),
        EvmProviderResponse::PossibleEntry {
            call_id,
            operation,
            candidate_id,
        } => call_id
            .as_str()
            .len()
            .saturating_add(operation.as_str().len())
            .saturating_add(candidate_id.len()),
    };
    size <= MAX_EVM_PROVIDER_RESPONSE_BYTES
}

fn valid_binding_text(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}
