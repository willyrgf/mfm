use std::marker::PhantomData;

use mfm_canonical::CanonicalBytes;
use mfm_capabilities::{CapabilityError, ReadCapabilityContract};
use mfm_ids::{ContentRef, StableId};
use mfm_program::{
    CapabilityInjection, PreparationError, ProgramError, ProposedStateOutcome, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::MfmValue as MfmValueTrait;
use serde::de;
use serde::{Deserialize, Serialize};

use crate::{
    EvmAddress, EvmBlockAnchor, EvmDomainError, EvmReadEvidence, EvmReadIntent, EvmReadSubject,
    EvmReadValue, EvmTransactionRoute, MAX_EVM_CALLDATA_BYTES,
};

/// Exact anchored call operation identity retained in [`EvmReadIntent`].
pub const EVM_ANCHORED_CONTRACT_CALL_OPERATION_ID: &str = "mfm.evm.read-anchored-contract-call@1";
/// Exact anchored contract-call Read capability identity.
pub const EVM_ANCHORED_CONTRACT_CALL_CAPABILITY_ID: &str =
    "mfm.evm.capability.read-anchored-contract-call@1";
/// Exact anchored contract-call State identity.
pub const READ_ANCHORED_CONTRACT_CALL_STATE_ID: &str =
    "mfm.evm.state.read-anchored-contract-call@1";
/// Maximum returned bytes retained by an anchored call.
pub const MAX_EVM_CALL_RETURN_BYTES: usize = 131_072;

/// Exact anchor and bounded bytes returned by one contract call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-contract-call-result",
    version = "1",
    schema = "mfm.evm-anchored-contract-call-result"
)]
pub struct AnchoredContractCallResult {
    anchor: EvmBlockAnchor,
    #[mfm(minimum_bytes = 0, maximum_bytes = 131072)]
    return_bytes: CanonicalBytes,
}

impl AnchoredContractCallResult {
    /// Constructs an exact anchored result from bounded public bytes.
    pub fn new(anchor: EvmBlockAnchor, return_bytes: Vec<u8>) -> Result<Self, EvmDomainError> {
        if return_bytes.len() > MAX_EVM_CALL_RETURN_BYTES {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self {
            anchor,
            return_bytes: CanonicalBytes::new(return_bytes),
        })
    }

    /// Returns the exact block anchor.
    pub const fn anchor(&self) -> &EvmBlockAnchor {
        &self.anchor
    }

    /// Returns the bounded call return bytes.
    pub fn return_bytes(&self) -> &[u8] {
        self.return_bytes.as_bytes()
    }

    pub(crate) fn validate(&self) -> Result<(), EvmDomainError> {
        (self.return_bytes.as_bytes().len() <= MAX_EVM_CALL_RETURN_BYTES)
            .then_some(())
            .ok_or(EvmDomainError::InvalidValue)
    }
}

impl<'de> Deserialize<'de> for AnchoredContractCallResult {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            anchor: EvmBlockAnchor,
            return_bytes: CanonicalBytes,
        }

        let wire = Wire::deserialize(deserializer)?;
        let value = Self {
            anchor: wire.anchor,
            return_bytes: wire.return_bytes,
        };
        value.validate().map(|_| value).map_err(de::Error::custom)
    }
}

/// Closed failure reason of one anchored contract-call Read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-contract-call-failure-reason",
    version = "1",
    schema = "mfm.evm-anchored-contract-call-failure-reason"
)]
pub enum AnchoredContractCallFailureReason {
    /// The anchored target had no deployed code.
    Rejected,
    /// The named anchor was absent in an authenticated observation.
    SafeFailure,
    /// Authenticated observations disagreed with the authored anchor.
    IntegrityBlocked,
}

impl<'de> Deserialize<'de> for AnchoredContractCallFailureReason {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Kind {
            Rejected,
            SafeFailure,
            IntegrityBlocked,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: Kind,
        }

        Ok(match Wire::deserialize(deserializer)?.kind {
            Kind::Rejected => Self::Rejected,
            Kind::SafeFailure => Self::SafeFailure,
            Kind::IntegrityBlocked => Self::IntegrityBlocked,
        })
    }
}

/// Caller context paired with one exact anchored call intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-contract-call-context",
    version = "1",
    schema = "mfm.evm-anchored-contract-call-context"
)]
pub struct AnchoredContractCallContext<K: MfmValueTrait> {
    caller_context: K,
    intent: EvmReadIntent,
}

impl<'de, K: MfmValueTrait> Deserialize<'de> for AnchoredContractCallContext<K> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        struct Wire<K> {
            caller_context: K,
            intent: EvmReadIntent,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.caller_context, wire.intent).map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> AnchoredContractCallContext<K> {
    /// Constructs one context after checking the exact anchored-call intent contract.
    pub fn new(caller_context: K, intent: EvmReadIntent) -> Result<Self, EvmDomainError> {
        intent.validate_anchored_contract_call()?;
        Ok(Self {
            caller_context,
            intent,
        })
    }

    /// Constructs the exact intent for a transaction route and authored anchor.
    pub fn for_route(
        caller_context: K,
        route: &EvmTransactionRoute,
        target: EvmAddress,
        calldata: Vec<u8>,
        anchor: EvmBlockAnchor,
    ) -> Result<Self, EvmDomainError> {
        if calldata.len() > MAX_EVM_CALLDATA_BYTES {
            return Err(EvmDomainError::InvalidValue);
        }
        let intent = EvmReadIntent::new(
            EVM_ANCHORED_CONTRACT_CALL_OPERATION_ID.to_owned(),
            route.chain_instance().chain_id(),
            EvmReadSubject::AnchoredContractCall {
                anchor,
                calldata: CanonicalBytes::new(calldata),
                target,
            },
            route.binding_ref()?,
        )?;
        Self::new(caller_context, intent)
    }

    /// Returns the unchanged caller context.
    pub const fn caller_context(&self) -> &K {
        &self.caller_context
    }

    /// Returns the exact anchored call intent.
    pub const fn intent(&self) -> &EvmReadIntent {
        &self.intent
    }
}

/// Caller context and successful anchored call result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-contract-call-completion",
    version = "1",
    schema = "mfm.evm-anchored-contract-call-completion"
)]
pub struct AnchoredContractCallCompletion<K: MfmValueTrait> {
    caller_context: K,
    result: AnchoredContractCallResult,
}

impl<K: MfmValueTrait> AnchoredContractCallCompletion<K> {
    /// Returns the unchanged caller context.
    pub const fn caller_context(&self) -> &K {
        &self.caller_context
    }

    /// Returns the exact anchored result.
    pub const fn result(&self) -> &AnchoredContractCallResult {
        &self.result
    }
}

/// Caller context and closed anchored call failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-contract-call-failure",
    version = "1",
    schema = "mfm.evm-anchored-contract-call-failure"
)]
pub struct AnchoredContractCallFailure<K: MfmValueTrait> {
    caller_context: K,
    reason: AnchoredContractCallFailureReason,
}

impl<K: MfmValueTrait> AnchoredContractCallFailure<K> {
    /// Returns the unchanged caller context.
    pub const fn caller_context(&self) -> &K {
        &self.caller_context
    }

    /// Returns the closed redaction-safe reason.
    pub const fn reason(&self) -> AnchoredContractCallFailureReason {
        self.reason
    }
}

/// Duplicate-safe anchored contract-call Read capability.
pub struct EvmAnchoredContractCallRead;

impl ReadCapabilityContract for EvmAnchoredContractCallRead {
    type Intent = EvmReadIntent;
    type Evidence = EvmReadEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new(EVM_ANCHORED_CONTRACT_CALL_CAPABILITY_ID)
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        _intent_value_ref: &ContentRef,
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        intent
            .validate_anchored_contract_call()
            .and_then(|_| evidence.validate_for(intent))
            .map_err(|_| CapabilityError::EvidenceBinding)
    }
}

/// Context-preserving anchored contract-call Read State.
pub struct ReadAnchoredContractCall<K: MfmValueTrait>(PhantomData<fn() -> K>);

impl<K: MfmValueTrait> State for ReadAnchoredContractCall<K> {
    type Input = AnchoredContractCallContext<K>;
    type Output = AnchoredContractCallCompletion<K>;
    type Failure = AnchoredContractCallFailure<K>;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new(READ_ANCHORED_CONTRACT_CALL_STATE_ID)
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl<K: MfmValueTrait> ReadState<EvmAnchoredContractCallRead> for ReadAnchoredContractCall<K> {
    fn prepare(input: &Self::Input) -> Result<EvmReadIntent, PreparationError> {
        input
            .intent
            .validate_anchored_contract_call()
            .map(|_| input.intent.clone())
            .map_err(|_| PreparationError)
    }

    fn interpret(
        input: Self::Input,
        evidence: &EvmReadEvidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        let reason = match evidence {
            EvmReadEvidence::Returned {
                value: EvmReadValue::AnchoredContractCall(result),
            } if evidence.validate_for(&input.intent).is_ok() => {
                return ProposedStateOutcome::Success {
                    output: AnchoredContractCallCompletion {
                        caller_context: input.caller_context,
                        result: result.clone(),
                    },
                };
            }
            EvmReadEvidence::Rejected => AnchoredContractCallFailureReason::Rejected,
            EvmReadEvidence::SafeFailure => AnchoredContractCallFailureReason::SafeFailure,
            EvmReadEvidence::IntegrityBlocked | EvmReadEvidence::Returned { .. } => {
                AnchoredContractCallFailureReason::IntegrityBlocked
            }
        };
        ProposedStateOutcome::Failure {
            failure: AnchoredContractCallFailure {
                caller_context: input.caller_context,
                reason,
            },
        }
    }
}

impl<K: MfmValueTrait> CapabilityInjection<ReadAnchoredContractCall<K>>
    for EvmAnchoredContractCallRead
{
    type Setup = EvmTransactionRoute;
    type ExpandedInput = AnchoredContractCallContext<K>;
    type ExpandedOutput = AnchoredContractCallCompletion<K>;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
}
