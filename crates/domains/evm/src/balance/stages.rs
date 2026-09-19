use std::{marker::PhantomData, num::NonZeroU64};

use mfm_chain::balance::{
    BalanceCollectionFailure, BalanceContext, CandidateBalance, DecimalScale, PreparedBalance,
};
use mfm_chain::ObservationPoint;
use mfm_ids::StableId;
use mfm_program::{
    Classification, ClassifyError, ProposedStateOutcome, ReadSelection, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::{InvocationDiagnostic, MfmValue as Value, Object};
use serde::{Deserialize, Serialize};

use crate::{
    EvmAnchorRead, EvmBalanceRead, EvmBlockAnchor, EvmBlockPoint, EvmChainIdentityRead,
    EvmReadEvidence, EvmReadIntent, EvmReadSubject, EvmReadValue,
};

/// Authenticated native rejection reason, independent of caller presentation fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
pub enum EvmObservationRejection {
    /// The native protocol authenticated rejection.
    Rejected,
    /// The native protocol authenticated absence.
    SafeFailure,
    /// The endpoint returned a different chain identity.
    ChainMismatch,
}

/// Native-currency balance implementation selector and fixed preparation specialization.
pub struct EvmNativeBalance;
/// Token balance implementation selector and fixed preparation specialization.
pub struct EvmTokenBalance;

fn local(operation: &'static str) -> InvocationDiagnostic {
    InvocationDiagnostic::from_fields(
        "state_internal",
        operation,
        &mfm_capabilities::CapabilityError::EvidenceBinding,
        None,
    )
}
fn shared_point<K: Value>(
    context: &BalanceContext<K>,
    anchor: &EvmBlockAnchor,
) -> Result<ObservationPoint, InvocationDiagnostic> {
    let source = context.active_source().map_err(|source| {
        InvocationDiagnostic::from_fields(
            "state_internal",
            "balance_observation_point",
            &source,
            None,
        )
    })?;
    Ok(ObservationPoint::new(
        source.target().ledger().clone(),
        Object::from_value(&EvmBlockPoint::new(
            anchor.number.clone(),
            anchor.hash.clone(),
        ))
        .map_err(|source| source.into_diagnostic("balance_observation_point"))?,
    ))
}
fn native_anchor(point: &ObservationPoint) -> Result<EvmBlockAnchor, InvocationDiagnostic> {
    let native = point.native().decode::<EvmBlockPoint>()?;
    Ok(EvmBlockAnchor {
        number: native.number().clone(),
        hash: native.hash().clone(),
    })
}
fn changed_anchor<K: Value>(
    context: &BalanceContext<K>,
    observed: &EvmBlockAnchor,
) -> Result<Option<EvmBalanceFailure>, InvocationDiagnostic> {
    if let Some(previous) = context.completed().first() {
        let previous = native_anchor(previous.observed_at())?;
        if &previous != observed {
            return Ok(Some(EvmBalanceFailure::AnchorChanged {
                previous,
                observed: observed.clone(),
            }));
        }
    }
    Ok(None)
}

/// Token context after chain and anchor checks, before observing token decimals.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmTokenAnchored<K: Value> {
    checked: EvmChainChecked<K>,
    anchor: EvmBlockAnchor,
}
impl<K: Value> EvmTokenAnchored<K> {
    /// Checks token qualification and agreement with the retained common collection point.
    pub fn new(
        checked: EvmChainChecked<K>,
        anchor: EvmBlockAnchor,
    ) -> Result<Self, InvocationDiagnostic> {
        EvmReadIntent::from_context(
            checked.context(),
            EvmReadSubject::TokenDecimals {
                anchor: anchor.clone(),
            },
        )?;
        if let Some(source) = changed_anchor(checked.context(), &anchor)? {
            return Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "construct_token_anchor",
                &source,
                None,
            ));
        }
        Ok(Self { checked, anchor })
    }
    /// Checked collection and chain facts.
    pub fn checked(&self) -> &EvmChainChecked<K> {
        &self.checked
    }
    /// Exact committed anchor for decimals and balance observations.
    pub fn anchor(&self) -> &EvmBlockAnchor {
        &self.anchor
    }
}
impl<'de, K: Value> Deserialize<'de> for EvmTokenAnchored<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        struct Wire<K: Value> {
            checked: EvmChainChecked<K>,
            anchor: EvmBlockAnchor,
        }
        let wire = Wire::<K>::deserialize(deserializer)?;
        Self::new(wire.checked, wire.anchor)
            .map_err(|error| serde::de::Error::custom(error.details().as_value()))
    }
}

/// Reads the initial anchor with exact native/token preparation output selected at construction.
pub struct ReadInitialEvmBalanceAnchor<K, I>(PhantomData<fn() -> (K, I)>);
fn prepare_native<K: Value>(
    checked: EvmChainChecked<K>,
    anchor: EvmBlockAnchor,
) -> Result<PreparedBalance<K>, InvocationDiagnostic> {
    let point = shared_point(&checked.context, &anchor)?;
    let scale = checked.context.request().decimals();
    PreparedBalance::new(checked.context, point, scale).map_err(|source| {
        InvocationDiagnostic::from_fields("state_internal", "prepare_native_balance", &source, None)
    })
}
macro_rules! initial_anchor {
    ($mode:ty, $token:literal, $output:ty, $construct:expr) => {
        impl<K: Value> ReadSelection<EvmAnchorRead> for ReadInitialEvmBalanceAnchor<K, $mode> {
            type ExpandedInput = EvmChainChecked<K>;
            type ExpandedOutput = $output;
        }
        impl<K: Value> ReadState<EvmAnchorRead> for ReadInitialEvmBalanceAnchor<K, $mode> {
            fn prepare(input: &EvmChainChecked<K>) -> Result<EvmReadIntent, InvocationDiagnostic> {
                let intent =
                    EvmReadIntent::from_context(input.context(), EvmReadSubject::InitialAnchor)?;
                if intent.target().token().is_some() != $token {
                    return Err(local("prepare_balance_anchor_asset"));
                }
                Ok(intent)
            }
            fn interpret(
                input: EvmChainChecked<K>,
                evidence: &EvmReadEvidence,
            ) -> Result<ProposedStateOutcome<$output, EvmBalanceFailure>, InvocationDiagnostic>
            {
                let intent = Self::prepare(&input)?;
                let value = match returned(&intent, evidence)? {
                    Ok(value) => value,
                    Err(failure) => return Ok(ProposedStateOutcome::Failure { failure }),
                };
                let EvmReadValue::Anchor(anchor) = value else {
                    return Err(local("interpret_initial_balance_anchor"));
                };
                if let Some(failure) = changed_anchor(input.context(), anchor)? {
                    return Ok(ProposedStateOutcome::Failure { failure });
                }
                Ok(ProposedStateOutcome::Success {
                    output: ($construct)(input, anchor.clone())?,
                })
            }
        }
    };
}
initial_anchor!(EvmNativeBalance, false, PreparedBalance<K>, prepare_native);
initial_anchor!(
    EvmTokenBalance,
    true,
    EvmTokenAnchored<K>,
    EvmTokenAnchored::new
);

/// Reads token decimals at the initial anchor before the designated raw balance observation.
pub struct ReadEvmTokenDecimals<K>(PhantomData<fn() -> K>);
impl<K: Value> ReadSelection<EvmBalanceRead> for ReadEvmTokenDecimals<K> {
    type ExpandedInput = EvmTokenAnchored<K>;
    type ExpandedOutput = PreparedBalance<K>;
}
impl<K: Value> ReadState<EvmBalanceRead> for ReadEvmTokenDecimals<K> {
    fn prepare(input: &EvmTokenAnchored<K>) -> Result<EvmReadIntent, InvocationDiagnostic> {
        EvmReadIntent::from_context(
            input.checked.context(),
            EvmReadSubject::TokenDecimals {
                anchor: input.anchor.clone(),
            },
        )
    }
    fn interpret(
        input: EvmTokenAnchored<K>,
        evidence: &EvmReadEvidence,
    ) -> Result<ProposedStateOutcome<PreparedBalance<K>, EvmBalanceFailure>, InvocationDiagnostic>
    {
        let intent = Self::prepare(&input)?;
        let value = match returned(&intent, evidence)? {
            Ok(value) => value,
            Err(failure) => return Ok(ProposedStateOutcome::Failure { failure }),
        };
        let EvmReadValue::TokenDecimals(decimals) = value else {
            return Err(local("interpret_balance_decimals"));
        };
        let scale = DecimalScale::new(decimals.get()).map_err(|source| {
            InvocationDiagnostic::from_fields(
                "state_internal",
                "interpret_balance_decimals",
                &source,
                None,
            )
        })?;
        let point = shared_point(input.checked.context(), &input.anchor)?;
        let output =
            PreparedBalance::new(input.checked.context, point, scale).map_err(|source| {
                InvocationDiagnostic::from_fields(
                    "state_internal",
                    "prepare_token_balance",
                    &source,
                    None,
                )
            })?;
        Ok(ProposedStateOutcome::Success { output })
    }
}

/// Re-observes the committed block number and admits an amount only after matching confirmation.
pub struct ConfirmEvmBalanceAnchor<K>(PhantomData<fn() -> K>);
impl<K: Value> ReadSelection<EvmAnchorRead> for ConfirmEvmBalanceAnchor<K> {
    type ExpandedInput = CandidateBalance<K>;
    type ExpandedOutput = BalanceContext<K>;
}
impl<K: Value> ReadState<EvmAnchorRead> for ConfirmEvmBalanceAnchor<K> {
    fn prepare(input: &CandidateBalance<K>) -> Result<EvmReadIntent, InvocationDiagnostic> {
        let prepared = input.prepared();
        let anchor = native_anchor(prepared.observed_at())?;
        let intent = EvmReadIntent::from_context(
            prepared.context(),
            EvmReadSubject::ConfirmAnchor { anchor },
        )?;
        if intent.target().token().is_none()
            && prepared.source_decimals() != intent.collection_scale()
        {
            return Err(local("confirm_native_balance_scale"));
        }
        Ok(intent)
    }
    fn interpret(
        input: CandidateBalance<K>,
        evidence: &EvmReadEvidence,
    ) -> Result<ProposedStateOutcome<BalanceContext<K>, EvmBalanceFailure>, InvocationDiagnostic>
    {
        let intent = Self::prepare(&input)?;
        let value = match returned(&intent, evidence)? {
            Ok(value) => value,
            Err(failure) => return Ok(ProposedStateOutcome::Failure { failure }),
        };
        let EvmReadValue::Anchor(observed) = value else {
            return Err(local("interpret_confirm_balance_anchor"));
        };
        let previous = native_anchor(input.prepared().observed_at())?;
        if &previous != observed {
            return Ok(ProposedStateOutcome::Failure {
                failure: EvmBalanceFailure::AnchorChanged {
                    previous,
                    observed: observed.clone(),
                },
            });
        }
        Ok(match input.append_confirmed()? {
            Ok(output) => ProposedStateOutcome::Success { output },
            Err(source) => ProposedStateOutcome::Failure {
                failure: EvmBalanceFailure::Collection { source },
            },
        })
    }
}

/// Exact native balance failure; the State call retains input and original evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue, thiserror::Error)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-failure",
    version = "2",
    schema = "mfm.evm-balance-failure"
)]
pub enum EvmBalanceFailure {
    /// Authenticated anchors differ; retry must invalidate the previous input.
    #[non_exhaustive]
    #[error("balance anchor changed")]
    AnchorChanged {
        /// Exact previously admitted anchor.
        previous: EvmBlockAnchor,
        /// Exact authenticated conflicting observation.
        observed: EvmBlockAnchor,
    },
    /// Authenticated unsuccessful native observation.
    #[error("balance observation rejected: {reason:?}")]
    ObservationRejected {
        /// Native reason retained without caller ordinal or presentation code.
        reason: EvmObservationRejection,
    },
    /// Authenticated evidence violated integrity constraints.
    #[error("balance observation integrity blocked")]
    IntegrityBlocked,
    /// Arithmetic rejected a candidate after successful native confirmation.
    #[error("balance collection amount rejected: {source}")]
    Collection {
        /// Exact semantic arithmetic failure and its complete causes.
        #[source]
        source: BalanceCollectionFailure,
    },
}
impl ClassifyError for EvmBalanceFailure {
    fn classify(&self) -> Classification {
        match self {
            Self::AnchorChanged { .. } => Classification::InputInvalidated,
            Self::ObservationRejected { .. } | Self::IntegrityBlocked | Self::Collection { .. } => {
                Classification::Permanent
            }
        }
    }
}
impl<'de> Deserialize<'de> for EvmBalanceFailure {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            AnchorChanged {
                previous: EvmBlockAnchor,
                observed: EvmBlockAnchor,
            },
            ObservationRejected {
                reason: EvmObservationRejection,
            },
            IntegrityBlocked,
            Collection {
                source: BalanceCollectionFailure,
            },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::AnchorChanged { previous, observed } if previous != observed => {
                Self::AnchorChanged { previous, observed }
            }
            Wire::AnchorChanged { .. } => {
                return Err(serde::de::Error::custom("changed anchors must differ"))
            }
            Wire::ObservationRejected { reason } => Self::ObservationRejected { reason },
            Wire::IntegrityBlocked => Self::IntegrityBlocked,
            Wire::Collection { source } => Self::Collection { source },
        })
    }
}

/// Context after an authenticated matching chain observation.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmChainChecked<K: Value> {
    context: BalanceContext<K>,
    checked_chain_id: NonZeroU64,
}
impl<K: Value> EvmChainChecked<K> {
    /// Checks retained chain facts against the active source; this is not provider authentication.
    pub fn new(
        context: BalanceContext<K>,
        checked_chain_id: NonZeroU64,
    ) -> Result<Self, InvocationDiagnostic> {
        let intent = EvmReadIntent::from_context(&context, EvmReadSubject::ChainIdentity)?;
        if intent.chain_id() != checked_chain_id {
            #[derive(Debug, Serialize, thiserror::Error)]
            #[error("retained chain observation does not match the active source")]
            struct ChainMismatch {
                expected: NonZeroU64,
                observed: NonZeroU64,
            }
            return Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "construct_chain_checked",
                &ChainMismatch {
                    expected: intent.chain_id(),
                    observed: checked_chain_id,
                },
                None,
            ));
        }
        Ok(Self {
            context,
            checked_chain_id,
        })
    }
    /// Complete shared collection context, without duplicated caller metadata.
    pub fn context(&self) -> &BalanceContext<K> {
        &self.context
    }
    /// Exact matching native chain observation.
    pub fn checked_chain_id(&self) -> NonZeroU64 {
        self.checked_chain_id
    }
}
impl<'de, K: Value> Deserialize<'de> for EvmChainChecked<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        struct Wire<K: Value> {
            context: BalanceContext<K>,
            checked_chain_id: NonZeroU64,
        }
        let wire = Wire::<K>::deserialize(deserializer)?;
        Self::new(wire.context, wire.checked_chain_id)
            .map_err(|error| serde::de::Error::custom(error.details().as_value()))
    }
}

fn returned<'a>(
    intent: &EvmReadIntent,
    evidence: &'a EvmReadEvidence,
) -> Result<Result<&'a EvmReadValue, EvmBalanceFailure>, InvocationDiagnostic> {
    let intent_object = Object::from_value(intent)
        .map_err(|source| source.into_diagnostic("native_balance_intent_identity"))?;
    if evidence.intent_value_ref() != intent_object.value_ref() {
        return Err(InvocationDiagnostic::from_fields(
            "state_internal",
            "native_balance_intent_binding",
            &mfm_capabilities::CapabilityError::EvidenceBinding,
            None,
        ));
    }
    evidence.validate_for(intent).map_err(|source| {
        InvocationDiagnostic::from_fields(
            "state_internal",
            "native_balance_evidence_kind",
            &source,
            None,
        )
    })?;
    Ok(match evidence {
        EvmReadEvidence::Returned { value, .. } => Ok(value),
        EvmReadEvidence::Rejected { .. } => Err(EvmBalanceFailure::ObservationRejected {
            reason: EvmObservationRejection::Rejected,
        }),
        EvmReadEvidence::SafeFailure { .. } => Err(EvmBalanceFailure::ObservationRejected {
            reason: EvmObservationRejection::SafeFailure,
        }),
        EvmReadEvidence::IntegrityBlocked { .. } => Err(EvmBalanceFailure::IntegrityBlocked),
    })
}

/// Checks the selected endpoint's chain before every source's anchor observation.
pub struct CheckEvmBalanceChain<K>(PhantomData<fn() -> K>);
impl<K: Value> ReadSelection<EvmChainIdentityRead> for CheckEvmBalanceChain<K> {
    type ExpandedInput = BalanceContext<K>;
    type ExpandedOutput = EvmChainChecked<K>;
}
impl<K: Value> ReadState<EvmChainIdentityRead> for CheckEvmBalanceChain<K> {
    fn prepare(input: &BalanceContext<K>) -> Result<EvmReadIntent, InvocationDiagnostic> {
        EvmReadIntent::from_context(input, EvmReadSubject::ChainIdentity)
    }
    fn interpret(
        input: BalanceContext<K>,
        evidence: &EvmReadEvidence,
    ) -> Result<ProposedStateOutcome<EvmChainChecked<K>, EvmBalanceFailure>, InvocationDiagnostic>
    {
        let intent = Self::prepare(&input)?;
        let value = match returned(&intent, evidence)? {
            Ok(value) => value,
            Err(failure) => return Ok(ProposedStateOutcome::Failure { failure }),
        };
        match value {
            EvmReadValue::ChainId(chain) if *chain == intent.chain_id() => {
                Ok(ProposedStateOutcome::Success {
                    output: EvmChainChecked::new(input, *chain)?,
                })
            }
            EvmReadValue::ChainId(_) => Ok(ProposedStateOutcome::Failure {
                failure: EvmBalanceFailure::ObservationRejected {
                    reason: EvmObservationRejection::ChainMismatch,
                },
            }),
            _ => Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "interpret_balance_chain",
                &mfm_capabilities::CapabilityError::EvidenceBinding,
                None,
            )),
        }
    }
}

// Each declaration owns both its executable State identity and the corresponding native input
// decoder. Generic caller specializations share metadata, but never substitute for exact ABIs.
macro_rules! balance_states {
    ($(($stage:ident, $state:ty, $input:ty, $output:ty, $capability:ty, $id:literal, $description:literal, $value:ident => $context:expr)),+ $(,)?) => {
        /// Native balance stage derived from its owning State declarations.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum NativeBalanceStage { $(#[doc = $description] $stage,)+ }
        $(impl<K: Value> State for $state {
            type Input = $input;
            type Output = $output;
            type Failure = EvmBalanceFailure;
            fn state_id() -> mfm_program::Result<StableId> { Ok(StableId::new($id)?) }
            fn description() -> &'static str { $description }
        })+

        /// Decodes a native balance State's exact retained input and visits its shared context.
        /// Unknown State identities return None; a known identity with a different ABI is rejected.
        /// The caller supplies only a synchronous semantic projection, never an execution hook.
        pub fn project_balance_context<K: Value, T>(
            declaration: &mfm_program::StateDeclaration,
            input: &Object,
            project: impl FnOnce(NativeBalanceStage, &BalanceContext<K>) -> Result<T, InvocationDiagnostic>,
        ) -> Result<Option<T>, InvocationDiagnostic> {
            let decode = || -> mfm_program::Result<Option<T>> {
                $(if declaration.state_implementation_ref() == &mfm_program::state_implementation_ref::<$state>()? {
                    if declaration.input_contract_ref() != &mfm_program::nominal_contract_ref::<$input>()?
                        || declaration.output_contract_ref() != &mfm_program::nominal_contract_ref::<$output>()?
                        || declaration.failure_contract_ref() != &mfm_program::nominal_contract_ref::<EvmBalanceFailure>()?
                        || !matches!(declaration.execution(), mfm_program::Execution::Read { abi, .. }
                            if abi == &mfm_program::NativeAbi::read::<$capability, super::EvmBalanceObservation>()?)
                    {
                        return Err(mfm_program::ProgramError::Diagnostic(local("project_native_balance_abi")));
                    }
                    let $value = input.decode::<$input>().map_err(mfm_program::ProgramError::Diagnostic)?;
                    return project(NativeBalanceStage::$stage, $context).map(Some).map_err(mfm_program::ProgramError::Diagnostic);
                })+
                Ok(None)
            };
            decode().map_err(|cause| match cause {
                mfm_program::ProgramError::Diagnostic(cause) => cause,
                cause => InvocationDiagnostic::from_fields("native_projection", "project_balance_context", &cause, None),
            })
        }
    };
}
balance_states!(
    (ChainIdentity, CheckEvmBalanceChain<K>, BalanceContext<K>, EvmChainChecked<K>, EvmChainIdentityRead,
     "mfm.evm.check-balance-chain@1", "Checks the native chain identity before collection IO.", value => &value),
    (NativeAnchor, ReadInitialEvmBalanceAnchor<K, EvmNativeBalance>, EvmChainChecked<K>, PreparedBalance<K>, EvmAnchorRead,
     "mfm.evm.initial-native-balance-anchor@1", "Selects the initial native-currency collection anchor.", value => value.context()),
    (TokenAnchor, ReadInitialEvmBalanceAnchor<K, EvmTokenBalance>, EvmChainChecked<K>, EvmTokenAnchored<K>, EvmAnchorRead,
     "mfm.evm.initial-token-balance-anchor@1", "Selects the initial token collection anchor.", value => value.context()),
    (TokenDecimals, ReadEvmTokenDecimals<K>, EvmTokenAnchored<K>, PreparedBalance<K>, EvmBalanceRead,
     "mfm.evm.read-token-decimals@1", "Observes native token decimals at the selected anchor.", value => value.checked().context()),
    (Confirmation, ConfirmEvmBalanceAnchor<K>, CandidateBalance<K>, BalanceContext<K>, EvmAnchorRead,
     "mfm.evm.confirm-balance-anchor@1", "Confirms the native anchor before accepting a balance.", value => value.prepared().context()),
);
