#![warn(missing_docs)]
//! Qualified EVM adapter ingress and concrete live assembly registration.
//!
//! The module owns only observational provider entry. Domain State semantics stay in `mfm-evm`
//! and `mfm-portfolio`; each adapter receives a one-use committed Runtime call and derives every
//! request from its sealed intent and immutable binding descriptor.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_capabilities::AccessCapabilityContract;
use mfm_evm::{
    EvmAccessState, EvmBalanceBindings, EvmCapability, EvmPureState, EvmReadEvidence,
    EvmReadIntent, EvmReadValue, EvmState,
};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId, StableId};
use mfm_portfolio::{PortfolioContinuation, PortfolioPureState, PortfolioState};
use mfm_program::{BindingDescriptor, State};
use mfm_runtime::{
    AccessResolution, BoxFuture, CommittedCall, PreparationError, PureImplementation,
    RuntimeAssemblyBuilder, RuntimeError, UnresolvedClassification,
};
use serde::{Deserialize, Serialize};

const MAX_EVM_PROVIDER_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_EVM_REQUEST_BYTES: usize = 512 * 1024;

const EVM_LIVE_ADAPTER_ID: &str = "mfm.evm.live-adapter@1";

/// Returns the stable implementation identity for the concrete EVM live adapter.
pub fn evm_live_adapter_implementation_ref() -> Result<ContentRef, EvmAdapterError> {
    content_ref("mfm.adapter-implementation", EVM_LIVE_ADAPTER_ID.as_bytes())
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

#[derive(Debug, PartialEq, Eq)]
enum AuthenticatedReadResponse {
    Outcome(EvmReadEvidence),
    BlockedIntegrity(EvmReadEvidence),
}

/// One immutable observational adapter binding.
pub struct EvmAdapterBinding {
    descriptor: BindingDescriptor,
    target: EvmPhysicalTarget,
    provider: Arc<dyn EvmProvider>,
}

impl EvmAdapterBinding {
    /// Creates one Read binding with no signer, nonce authority, or mutation handle.
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
            provider,
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
        let intent = call.intent().clone();
        let (operation_name, chain_id) = intent.operation_and_chain_id();
        if chain_id != self.target.chain_id {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        if intent.route_ref() != self.descriptor.physical_target_ref() {
            return accept(call, EvmReadEvidence::IntegrityBlocked, true);
        }
        let operation =
            StableId::new(operation_name).map_err(|_| EvmAdapterError::Authentication)?;
        let request = intent_request_bytes(&call)?;
        let response = match self
            .provider
            .request(call.call_id().clone(), operation.clone(), request)
            .await
        {
            Ok(response) if response_within_bound(&response) => response,
            Ok(_) => return Ok(unresolved(call, UnresolvedClassification::InvalidResponse)),
            Err(_) => {
                return Ok(unresolved(
                    call,
                    UnresolvedClassification::AcknowledgementUnknown,
                ));
            }
        };
        let Some(authenticated) = authenticate_read_response(response, call.call_id(), &operation)
        else {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        };
        let (evidence, integrity) = match authenticated {
            AuthenticatedReadResponse::Outcome(evidence) => (evidence, false),
            AuthenticatedReadResponse::BlockedIntegrity(evidence) => (evidence, true),
        };
        if C::bind_evidence(&intent, &evidence).is_err() {
            return Ok(unresolved(call, UnresolvedClassification::InvalidResponse));
        }
        accept(call, evidence, integrity)
    }
}

fn authenticate_read_response(
    response: EvmProviderResponse,
    expected_call_id: &StableId,
    expected_operation: &StableId,
) -> Option<AuthenticatedReadResponse> {
    match response {
        EvmProviderResponse::Read {
            call_id,
            operation: response_operation,
            value,
        } if call_id == *expected_call_id && response_operation == *expected_operation => Some(
            AuthenticatedReadResponse::Outcome(EvmReadEvidence::Returned { value }),
        ),
        EvmProviderResponse::Rejected {
            call_id,
            operation: response_operation,
            code: _,
        } if call_id == *expected_call_id && response_operation == *expected_operation => Some(
            AuthenticatedReadResponse::Outcome(EvmReadEvidence::Rejected),
        ),
        EvmProviderResponse::SafeFailure {
            call_id,
            operation: response_operation,
            code: _,
        } if call_id == *expected_call_id && response_operation == *expected_operation => Some(
            AuthenticatedReadResponse::Outcome(EvmReadEvidence::SafeFailure),
        ),
        EvmProviderResponse::IntegrityBlocked {
            call_id,
            operation: response_operation,
            code: _,
        } if call_id == *expected_call_id && response_operation == *expected_operation => Some(
            AuthenticatedReadResponse::BlockedIntegrity(EvmReadEvidence::IntegrityBlocked),
        ),
        _ => None,
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

/// Exact planning bindings retained after one live EVM installation.
pub struct EvmLiveAssembly {
    balance_bindings: Vec<EvmBalanceBindings>,
}

impl EvmLiveAssembly {
    /// Validates and installs one complete finite descriptor closure into the Runtime builder.
    pub fn install(
        builder: &mut RuntimeAssemblyBuilder,
        balance_bindings: Vec<EvmBalanceBindings>,
        adapters: Vec<EvmAdapterBinding>,
    ) -> Result<Self, EvmAdapterError> {
        if balance_bindings.is_empty() {
            return Err(EvmAdapterError::Assembly);
        }
        let expected = balance_bindings
            .iter()
            .flat_map(|bindings| bindings.descriptors())
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
        if registered.len() != expected.len()
            || expected
                .iter()
                .any(|reference| !registered.contains_key(reference))
        {
            return Err(EvmAdapterError::Assembly);
        }
        if balance_bindings.iter().enumerate().any(|(index, binding)| {
            balance_bindings[..index]
                .iter()
                .any(|prior| prior.chain_id == binding.chain_id)
        }) {
            return Err(EvmAdapterError::Assembly);
        }
        for bindings in &balance_bindings {
            validate_balance_route(bindings, &registered)?;
        }
        register_all(builder, &balance_bindings, &registered)?;
        Ok(Self { balance_bindings })
    }

    /// Returns the exact balance bindings retained for domain planning.
    pub fn planning_bindings(&self) -> &[EvmBalanceBindings] {
        &self.balance_bindings
    }
}

fn register_all(
    builder: &mut RuntimeAssemblyBuilder,
    balance_bindings: &[EvmBalanceBindings],
    adapters: &BTreeMap<ContentRef, Arc<EvmAdapterBinding>>,
) -> Result<(), EvmAdapterError> {
    register_semantics(builder)?;
    for bindings in balance_bindings {
        let [check_chain_identity, read_initial_anchor, read_native_balance, read_token_decimals, read_token_balance, confirm_anchor] =
            bindings.descriptors();
        register_read::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<2>>(
            builder,
            adapters,
            check_chain_identity,
        )?;
        register_read::<EvmState<1, 1, PortfolioContinuation>, EvmCapability<6>>(
            builder,
            adapters,
            read_initial_anchor,
        )?;
        register_read::<EvmState<1, 3, PortfolioContinuation>, EvmCapability<7>>(
            builder,
            adapters,
            read_native_balance,
        )?;
        register_read::<EvmState<1, 4, PortfolioContinuation>, EvmCapability<7>>(
            builder,
            adapters,
            read_token_decimals,
        )?;
        register_read::<EvmState<1, 5, PortfolioContinuation>, EvmCapability<7>>(
            builder,
            adapters,
            read_token_balance,
        )?;
        register_read::<EvmState<1, 6, PortfolioContinuation>, EvmCapability<6>>(
            builder,
            adapters,
            confirm_anchor,
        )?;
    }
    Ok(())
}

macro_rules! register_adapter {
    ($name:ident, $intent:ty, $evidence:ty, $invoke:ident) => {
        fn $name<S, C>(
            builder: &mut RuntimeAssemblyBuilder,
            adapters: &BTreeMap<ContentRef, Arc<EvmAdapterBinding>>,
            descriptor: &BindingDescriptor,
        ) -> Result<(), EvmAdapterError>
        where
            S: State,
            C: AccessCapabilityContract<Intent = $intent, Evidence = $evidence>,
        {
            let adapter = registered_adapter(adapters, descriptor)?;
            builder
                .register_adapter::<S, C, _>(descriptor.clone(), move |call| {
                    let adapter = Arc::clone(&adapter);
                    Box::pin(async move {
                        adapter
                            .$invoke(call)
                            .await
                            .map_err(|_| RuntimeError::Unresolved)
                    })
                })
                .map_err(|_| EvmAdapterError::Assembly)
        }
    };
}

register_adapter!(register_read, EvmReadIntent, EvmReadEvidence, read_call);

fn registered_adapter(
    adapters: &BTreeMap<ContentRef, Arc<EvmAdapterBinding>>,
    descriptor: &BindingDescriptor,
) -> Result<Arc<EvmAdapterBinding>, EvmAdapterError> {
    let reference = descriptor
        .content_ref()
        .map_err(|_| EvmAdapterError::Assembly)?;
    adapters
        .get(&reference)
        .filter(|adapter| adapter.descriptor() == descriptor)
        .cloned()
        .ok_or(EvmAdapterError::Assembly)
}

fn validate_balance_route(
    bindings: &EvmBalanceBindings,
    adapters: &BTreeMap<ContentRef, Arc<EvmAdapterBinding>>,
) -> Result<(), EvmAdapterError> {
    for descriptor in bindings.descriptors() {
        let adapter = registered_adapter(adapters, descriptor)?;
        if adapter.target.chain_id != bindings.chain_id {
            return Err(EvmAdapterError::Assembly);
        }
    }
    Ok(())
}

fn register_semantics(builder: &mut RuntimeAssemblyBuilder) -> Result<(), EvmAdapterError> {
    register_evm_pure::<EvmState<1, 2, PortfolioContinuation>>(builder)?;
    register_evm_pure::<EvmState<1, 7, PortfolioContinuation>>(builder)?;
    register_portfolio_pure::<PortfolioState<0>>(builder)?;
    register_portfolio_pure::<PortfolioState<1>>(builder)?;
    register_portfolio_pure::<PortfolioState<2>>(builder)?;
    register_portfolio_pure::<PortfolioState<3>>(builder)?;
    register_portfolio_pure::<PortfolioState<4>>(builder)?;

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
        && (descriptor_has_role::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<2>>(
            descriptor,
        ) || descriptor_has_role::<EvmState<1, 1, PortfolioContinuation>, EvmCapability<6>>(
            descriptor,
        ) || descriptor_has_role::<EvmState<1, 3, PortfolioContinuation>, EvmCapability<7>>(
            descriptor,
        ) || descriptor_has_role::<EvmState<1, 4, PortfolioContinuation>, EvmCapability<7>>(
            descriptor,
        ) || descriptor_has_role::<EvmState<1, 5, PortfolioContinuation>, EvmCapability<7>>(
            descriptor,
        ) || descriptor_has_role::<EvmState<1, 6, PortfolioContinuation>, EvmCapability<6>>(
            descriptor,
        ))
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
    };
    size <= MAX_EVM_PROVIDER_RESPONSE_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correlation() -> (StableId, StableId) {
        (
            StableId::new("mfm.call.test").expect("call id"),
            StableId::new("mfm.evm.read-chain-identity@1").expect("operation"),
        )
    }

    #[test]
    fn every_surviving_response_maps_to_its_typed_read_resolution() {
        let (call_id, operation) = correlation();
        assert_eq!(
            authenticate_read_response(
                EvmProviderResponse::Read {
                    call_id: call_id.clone(),
                    operation: operation.clone(),
                    value: EvmReadValue::ChainId(1),
                },
                &call_id,
                &operation,
            ),
            Some(AuthenticatedReadResponse::Outcome(
                EvmReadEvidence::Returned {
                    value: EvmReadValue::ChainId(1),
                },
            )),
        );
        assert_eq!(
            authenticate_read_response(
                EvmProviderResponse::Rejected {
                    call_id: call_id.clone(),
                    operation: operation.clone(),
                    code: "rejected".to_owned(),
                },
                &call_id,
                &operation,
            ),
            Some(AuthenticatedReadResponse::Outcome(
                EvmReadEvidence::Rejected
            )),
        );
        assert_eq!(
            authenticate_read_response(
                EvmProviderResponse::SafeFailure {
                    call_id: call_id.clone(),
                    operation: operation.clone(),
                    code: "safe_failure".to_owned(),
                },
                &call_id,
                &operation,
            ),
            Some(AuthenticatedReadResponse::Outcome(
                EvmReadEvidence::SafeFailure,
            )),
        );
        assert_eq!(
            authenticate_read_response(
                EvmProviderResponse::IntegrityBlocked {
                    call_id: call_id.clone(),
                    operation: operation.clone(),
                    code: "integrity_blocked".to_owned(),
                },
                &call_id,
                &operation,
            ),
            Some(AuthenticatedReadResponse::BlockedIntegrity(
                EvmReadEvidence::IntegrityBlocked,
            )),
        );
    }

    #[test]
    fn read_response_mapping_requires_exact_call_and_operation_correlation() {
        let (call_id, operation) = correlation();
        assert_eq!(
            authenticate_read_response(
                EvmProviderResponse::Rejected {
                    call_id: StableId::new("mfm.call.other").expect("other call"),
                    operation: operation.clone(),
                    code: "rejected".to_owned(),
                },
                &call_id,
                &operation,
            ),
            None,
        );
        assert_eq!(
            authenticate_read_response(
                EvmProviderResponse::Rejected {
                    call_id: call_id.clone(),
                    operation: StableId::new("mfm.evm.other-operation@1").expect("other operation"),
                    code: "rejected".to_owned(),
                },
                &call_id,
                &operation,
            ),
            None,
        );
    }
}
