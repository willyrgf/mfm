#![warn(missing_docs)]
//! Qualified EVM adapter ingress.
//!
//! This crate owns provider transport and bounded protocol authentication.  It never exposes raw
//! response bytes or diagnostics to State code; callers receive only capability-owned evidence.

use std::sync::Arc;

use mfm_capabilities::{AccessCapabilityContract, CapabilityError};
use mfm_evm::{BroadcastEvidence, BroadcastIntent, EvmReadEvidence, EvmReadIntent};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId, StableId};
use mfm_runtime::{AccessResolution, BoxFuture, CommittedCall, State, UnresolvedClassification};
use serde::{Deserialize, Serialize};

/// Maximum provider response bytes admitted before decoding.
pub const MAX_EVM_PROVIDER_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// Maximum canonical request bytes derived from one committed intent.
pub const MAX_EVM_REQUEST_BYTES: usize = 512 * 1024;

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
    /// Validates the public route identity before it becomes an adapter binding.
    pub fn validate(&self) -> Result<(), EvmAdapterError> {
        if self.chain_id == 0 {
            return Err(EvmAdapterError::InvalidTarget);
        }
        Ok(())
    }

    /// Returns the content identity of the complete public target, including chain identity.
    pub fn content_ref(&self) -> Result<ContentRef, EvmAdapterError> {
        self.validate()?;
        let canonical = serde_json::to_string(self).map_err(|_| EvmAdapterError::InvalidTarget)?;
        let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&canonical)
            .map_err(|_| EvmAdapterError::InvalidTarget)?;
        let schema = SchemaId::new(
            "mfm.evm-physical-target",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .map_err(|_| EvmAdapterError::InvalidTarget)?;
        ContentRef::new(
            schema,
            mfm_canonical::raw_content_digest(canonical.as_bytes()),
        )
        .map_err(|_| EvmAdapterError::InvalidTarget)
    }
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
    /// The provider returned a reviewed rejection.
    #[error("EVM provider rejected the request")]
    Rejected,
    /// The provider outcome is unresolved and grants no generic retry authority.
    #[error("EVM provider outcome is unresolved")]
    Unresolved,
}

/// Bounded typed provider response after raw ingress has been discarded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmProviderResponse {
    /// Authenticated read value and anchor.
    Read {
        /// Call correlation echoed by the provider transport.
        call_id: StableId,
        /// Operation correlation echoed by the provider transport.
        operation: StableId,
        /// Bounded interpreted provider value.
        value: String,
        /// Provider anchor bound to the request.
        anchor: String,
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

/// One adapter binding containing only the provider and public route identity.
pub struct EvmAdapterBinding {
    /// Public route identity.
    target: EvmPhysicalTarget,
    state_implementation_ref: ContentRef,
    capability_contract_ref: ContentRef,
    execution_binding_ref: ContentRef,
    adapter_implementation_ref: ContentRef,
    sender: String,
    nonce_domain: String,
    effect_domain: Option<StableId>,
    public_signer_key_instance_ref: Option<ContentRef>,
    provider: Arc<dyn EvmProvider>,
}

impl EvmAdapterBinding {
    /// Constructs one immutable provider binding with the complete descriptor identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        target: EvmPhysicalTarget,
        state_implementation_ref: ContentRef,
        capability_contract_ref: ContentRef,
        execution_binding_ref: ContentRef,
        adapter_implementation_ref: ContentRef,
        sender: String,
        nonce_domain: String,
        effect_domain: Option<StableId>,
        public_signer_key_instance_ref: Option<ContentRef>,
        provider: Arc<dyn EvmProvider>,
    ) -> Result<Self, EvmAdapterError> {
        target.validate()?;
        if !valid_binding_text(&sender, 128)
            || sender != sender.to_ascii_lowercase()
            || !valid_binding_text(&nonce_domain, 256)
        {
            return Err(EvmAdapterError::InvalidTarget);
        }
        Ok(Self {
            target,
            state_implementation_ref,
            capability_contract_ref,
            execution_binding_ref,
            adapter_implementation_ref,
            sender,
            nonce_domain,
            effect_domain,
            public_signer_key_instance_ref,
            provider,
        })
    }

    /// Returns the provider-independent route identity.
    pub const fn target(&self) -> &EvmPhysicalTarget {
        &self.target
    }

    fn binding_matches<S: State, C: AccessCapabilityContract>(
        &self,
        call: &CommittedCall<S, C>,
    ) -> bool {
        let Ok(target_ref) = self.target.content_ref() else {
            return false;
        };
        call.execution_binding_ref() == &self.execution_binding_ref
            && call.binding().state_implementation_ref() == &self.state_implementation_ref
            && call.binding().capability_contract_ref() == Some(&self.capability_contract_ref)
            && call.binding().adapter_implementation_ref() == Some(&self.adapter_implementation_ref)
            && call.binding().physical_target_ref() == &target_ref
            && call.binding().effect_domain() == self.effect_domain.as_ref()
            && call.binding().public_signer_key_instance_ref()
                == self.public_signer_key_instance_ref.as_ref()
    }

    /// Performs a bounded read ingress and returns the closed adapter result algebra.
    pub async fn read_call<
        S: State,
        C: AccessCapabilityContract<Intent = EvmReadIntent, Evidence = EvmReadEvidence>,
    >(
        &self,
        call: CommittedCall<S, C>,
    ) -> Result<AccessResolution<S, C>, EvmAdapterError> {
        if !self.binding_matches(&call) {
            return Ok(AccessResolution::Unresolved(
                call.unresolved(UnresolvedClassification::InvalidResponse),
            ));
        }
        let intent = call.intent().clone();
        if intent.validate().is_err() || intent.chain_id != self.target.chain_id {
            return Ok(AccessResolution::Unresolved(
                call.unresolved(UnresolvedClassification::InvalidResponse),
            ));
        }
        let operation = match StableId::new(&intent.operation) {
            Ok(operation) => operation,
            Err(_) => {
                return Ok(AccessResolution::Unresolved(
                    call.unresolved(UnresolvedClassification::InvalidResponse),
                ))
            }
        };
        let request_bytes = match call.intent_canonical_bytes() {
            Ok(bytes) => bytes,
            Err(_) => {
                return Ok(AccessResolution::Unresolved(
                    call.unresolved(UnresolvedClassification::InvalidResponse),
                ))
            }
        };
        if request_bytes.len() > MAX_EVM_REQUEST_BYTES {
            return Ok(AccessResolution::Unresolved(
                call.unresolved(UnresolvedClassification::InvalidResponse),
            ));
        }
        let response = match self
            .provider
            .request(call.call_id().clone(), operation.clone(), request_bytes)
            .await
        {
            Ok(response) => response,
            Err(_) => {
                return Ok(AccessResolution::Unresolved(
                    call.unresolved(UnresolvedClassification::AcknowledgementUnknown),
                ))
            }
        };
        if !response_within_bound(&response) {
            return Ok(AccessResolution::Unresolved(
                call.unresolved(UnresolvedClassification::InvalidResponse),
            ));
        }
        let evidence = match response {
            EvmProviderResponse::Read {
                call_id,
                operation: response_operation,
                value,
                anchor,
            } if call_id == *call.call_id() && response_operation == operation => {
                EvmReadEvidence::Returned {
                    operation: intent.operation.clone(),
                    subject: intent.subject.clone(),
                    value,
                    anchor,
                }
            }
            EvmProviderResponse::Rejected {
                call_id,
                operation: response_operation,
                code,
            } if call_id == *call.call_id() && response_operation == operation => {
                EvmReadEvidence::Rejected {
                    operation: intent.operation.clone(),
                    code,
                }
            }
            EvmProviderResponse::SafeFailure {
                call_id,
                operation: response_operation,
                code,
            } if call_id == *call.call_id() && response_operation == operation => {
                EvmReadEvidence::SafeFailure {
                    operation: intent.operation.clone(),
                    code,
                }
            }
            EvmProviderResponse::IntegrityBlocked {
                call_id,
                operation: response_operation,
                code,
            } if call_id == *call.call_id() && response_operation == operation => {
                EvmReadEvidence::IntegrityBlocked {
                    operation: intent.operation.clone(),
                    code,
                }
            }
            _ => {
                return Ok(AccessResolution::Unresolved(
                    call.unresolved(UnresolvedClassification::InvalidResponse),
                ))
            }
        };
        if C::bind_evidence(&intent, &evidence).is_err() {
            return Ok(AccessResolution::Unresolved(
                call.unresolved(UnresolvedClassification::InvalidResponse),
            ));
        }
        let is_integrity = matches!(evidence, EvmReadEvidence::IntegrityBlocked { .. });
        if is_integrity {
            call.accept_integrity(evidence)
                .map(AccessResolution::BlockedIntegrity)
                .map_err(|_| EvmAdapterError::Authentication)
        } else {
            call.accept_evidence(evidence)
                .map(AccessResolution::Outcome)
                .map_err(|_| EvmAdapterError::Authentication)
        }
    }

    /// Performs the single one-entry broadcast ingress.
    pub async fn broadcast_call<
        S: State,
        C: AccessCapabilityContract<Intent = BroadcastIntent, Evidence = BroadcastEvidence>,
    >(
        &self,
        call: CommittedCall<S, C>,
    ) -> Result<AccessResolution<S, C>, EvmAdapterError> {
        if !self.binding_matches(&call) {
            return Ok(AccessResolution::Unresolved(
                call.unresolved(UnresolvedClassification::InvalidResponse),
            ));
        }
        let intent = call.intent().clone();
        if intent.validate().is_err()
            || intent.sender != self.sender
            || intent.nonce_domain != self.nonce_domain
        {
            return Ok(AccessResolution::Unresolved(
                call.unresolved(UnresolvedClassification::InvalidResponse),
            ));
        }
        let operation = StableId::new("mfm.evm.broadcast-transaction@1")
            .map_err(|_| EvmAdapterError::Authentication)?;
        let request_bytes = match call.intent_canonical_bytes() {
            Ok(bytes) => bytes,
            Err(_) => {
                return Ok(AccessResolution::Unresolved(
                    call.unresolved(UnresolvedClassification::InvalidResponse),
                ))
            }
        };
        if request_bytes.len() > MAX_EVM_REQUEST_BYTES {
            return Ok(AccessResolution::Unresolved(
                call.unresolved(UnresolvedClassification::InvalidResponse),
            ));
        }
        let response = match self
            .provider
            .request(call.call_id().clone(), operation.clone(), request_bytes)
            .await
        {
            Ok(response) => response,
            Err(_) => {
                return Ok(AccessResolution::Unresolved(
                    call.unresolved(UnresolvedClassification::AcknowledgementUnknown),
                ))
            }
        };
        if !response_within_bound(&response) {
            return Ok(AccessResolution::Unresolved(
                call.unresolved(UnresolvedClassification::InvalidResponse),
            ));
        }
        let evidence = match response {
            EvmProviderResponse::Broadcast {
                call_id,
                operation: response_operation,
                transaction_hash,
            } if call_id == *call.call_id() && response_operation == operation => {
                BroadcastEvidence::Returned {
                    candidate_id: intent.candidate_id.clone(),
                    transaction_hash,
                }
            }
            EvmProviderResponse::Rejected {
                call_id,
                operation: response_operation,
                code,
            } if call_id == *call.call_id() && response_operation == operation => {
                BroadcastEvidence::Rejected {
                    candidate_id: intent.candidate_id.clone(),
                    code,
                }
            }
            EvmProviderResponse::PossibleEntry {
                call_id,
                operation: response_operation,
                candidate_id,
            } if call_id == *call.call_id()
                && response_operation == operation
                && candidate_id == intent.candidate_id =>
            {
                return Ok(AccessResolution::Unresolved(
                    call.unresolved(UnresolvedClassification::AcknowledgementUnknown),
                ));
            }
            EvmProviderResponse::IntegrityBlocked {
                call_id,
                operation: response_operation,
                ..
            } if call_id == *call.call_id() && response_operation == operation => {
                BroadcastEvidence::IntegrityBlocked {
                    candidate_id: intent.candidate_id.clone(),
                }
            }
            _ => {
                return Ok(AccessResolution::Unresolved(
                    call.unresolved(UnresolvedClassification::InvalidResponse),
                ))
            }
        };
        if C::bind_evidence(&intent, &evidence).is_err() {
            return Ok(AccessResolution::Unresolved(
                call.unresolved(UnresolvedClassification::InvalidResponse),
            ));
        }
        if matches!(evidence, BroadcastEvidence::IntegrityBlocked { .. }) {
            call.accept_integrity(evidence)
                .map(AccessResolution::BlockedIntegrity)
                .map_err(|_| EvmAdapterError::Authentication)
        } else {
            call.accept_evidence(evidence)
                .map(AccessResolution::Outcome)
                .map_err(|_| EvmAdapterError::Authentication)
        }
    }
}

fn valid_binding_text(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

fn response_within_bound(response: &EvmProviderResponse) -> bool {
    let size = match response {
        EvmProviderResponse::Read {
            call_id,
            operation,
            value,
            anchor,
        } => call_id
            .as_str()
            .len()
            .saturating_add(operation.as_str().len())
            .saturating_add(value.len())
            .saturating_add(anchor.len()),
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

/// Converts a capability error into the sole redacted adapter fault.
pub const fn redact_capability_error(_: CapabilityError) -> EvmAdapterError {
    EvmAdapterError::Authentication
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correlations() -> (StableId, StableId) {
        (
            StableId::new("mfm.evm.call@1").expect("call id"),
            StableId::new("mfm.evm.read@1").expect("operation id"),
        )
    }

    #[test]
    fn every_definite_response_variant_is_bounded() {
        let (call_id, operation) = correlations();
        let responses = [
            EvmProviderResponse::Read {
                call_id: call_id.clone(),
                operation: operation.clone(),
                value: "0x01".to_owned(),
                anchor: "mfm.anchor@1".to_owned(),
            },
            EvmProviderResponse::Broadcast {
                call_id: call_id.clone(),
                operation: operation.clone(),
                transaction_hash: "0xabc".to_owned(),
            },
            EvmProviderResponse::Rejected {
                call_id: call_id.clone(),
                operation: operation.clone(),
                code: "rejected".to_owned(),
            },
            EvmProviderResponse::SafeFailure {
                call_id: call_id.clone(),
                operation: operation.clone(),
                code: "safe-failure".to_owned(),
            },
            EvmProviderResponse::PossibleEntry {
                call_id: call_id.clone(),
                operation: operation.clone(),
                candidate_id: "candidate-1".to_owned(),
            },
            EvmProviderResponse::IntegrityBlocked {
                call_id,
                operation,
                code: "integrity".to_owned(),
            },
        ];
        assert!(responses.iter().all(response_within_bound));
    }

    #[test]
    fn oversized_definite_response_is_rejected_before_interpretation() {
        let (call_id, operation) = correlations();
        let response = EvmProviderResponse::PossibleEntry {
            call_id,
            operation,
            candidate_id: "x".repeat(MAX_EVM_PROVIDER_RESPONSE_BYTES),
        };
        assert!(!response_within_bound(&response));
    }
}
