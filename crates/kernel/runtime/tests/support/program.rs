use mfm_capabilities::{CapabilityError, EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_program::{
    Classification, ClassifyError, EffectSelection, EffectState, Never, ProgramError,
    ProposedStateOutcome, PureState, ReadSelection, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::{canonicalize_mfm_value, InvocationDiagnostic};
use serde::{Deserialize, Serialize};

pub(super) const PREPARATION_FAILURE_SENTINEL: u64 = u64::MAX;

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct Number {
    pub(super) value: u64,
}

pub(super) struct Increment;

impl State for Increment {
    type Input = Number;
    type Output = Number;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/increment@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for Increment {
    fn evaluate(
        input: Self::Input,
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic>
    {
        Ok(ProposedStateOutcome::Success {
            output: Number {
                value: input.value + 1,
            },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    pub(super) value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct Evidence {
    pub(super) intent_value_ref: ContentRef,
    pub(super) value: u64,
    pub(super) accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub(super) route: u64,
}

pub(super) struct Observation;

impl ReadCapabilityContract for Observation {
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/observation@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent_value_ref: &ContentRef,
        intent: &Self::Intent,
        _: &ContentRef,
        evidence: &Self::Evidence,
    ) -> Result<(), InvocationDiagnostic> {
        let expected_value_ref = canonicalize_mfm_value(intent)
            .map(|(_, value_ref)| value_ref)
            .map_err(|cause| {
                InvocationDiagnostic::from_fields("state_internal", "bind_evidence", &cause, None)
            })?;
        (intent_value_ref == &expected_value_ref
            && intent_value_ref == &evidence.intent_value_ref
            && intent.value == evidence.value)
            .then_some(())
            .ok_or_else(|| {
                InvocationDiagnostic::from_fields(
                    "state_internal",
                    "bind_evidence",
                    &(CapabilityError::EvidenceBinding),
                    None,
                )
            })
    }
}

pub(super) struct Observe;

impl State for Observe {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/observe@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<Observation> for Observe {
    fn prepare(input: &Self::Input) -> Result<Intent, InvocationDiagnostic> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        evidence: &Evidence,
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic>
    {
        if evidence.accepted {
            Ok(ProposedStateOutcome::Success { output: input })
        } else {
            Ok(ProposedStateOutcome::Failure { failure: input })
        }
    }
}

impl ReadSelection<Observation> for Observe {
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct Command {
    pub(super) value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct EffectEvidence {
    pub(super) effect_id: EffectId,
    pub(super) value: u64,
    pub(super) accepted: bool,
}

pub(super) struct Mutation;

impl EffectCapabilityContract for Mutation {
    type Command = Command;
    type Evidence = EffectEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/mutation@1").map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        effect_id: &EffectId,
        _: &ContentRef,
        command: &Self::Command,
        _: &ContentRef,
        evidence: &Self::Evidence,
    ) -> Result<(), InvocationDiagnostic> {
        (effect_id == &evidence.effect_id && command.value == evidence.value)
            .then_some(())
            .ok_or_else(|| {
                InvocationDiagnostic::from_fields(
                    "state_internal",
                    "bind_evidence",
                    &(CapabilityError::EvidenceBinding),
                    None,
                )
            })
    }
}

pub(super) struct ConflictingReadCapability;

impl ReadCapabilityContract for ConflictingReadCapability {
    type Intent = Command;
    type Evidence = EffectEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Mutation::contract_id()
    }

    fn bind_evidence(
        _intent_value_ref: &ContentRef,
        intent: &Self::Intent,
        _: &ContentRef,
        evidence: &Self::Evidence,
    ) -> Result<(), InvocationDiagnostic> {
        (intent.value == evidence.value)
            .then_some(())
            .ok_or_else(|| {
                InvocationDiagnostic::from_fields(
                    "state_internal",
                    "bind_evidence",
                    &(CapabilityError::EvidenceBinding),
                    None,
                )
            })
    }
}

pub(super) struct Mutate;

#[derive(Debug, Serialize, thiserror::Error)]
#[error("test command preparation rejected input")]
struct PreparationRejected {
    input: u64,
}

#[derive(Debug, Serialize, thiserror::Error)]
#[error("test adapter rejected invocation")]
pub(super) struct AdapterRejected;

impl State for Mutate {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/mutate@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl EffectState<Mutation> for Mutate {
    fn prepare(input: &Self::Input) -> Result<Command, InvocationDiagnostic> {
        if input.value == PREPARATION_FAILURE_SENTINEL {
            return Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "prepare",
                &(PreparationRejected { input: input.value }),
                None,
            ));
        }
        Ok(Command { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        evidence: &EffectEvidence,
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic>
    {
        if evidence.accepted {
            Ok(ProposedStateOutcome::Success { output: input })
        } else {
            Ok(ProposedStateOutcome::Failure { failure: input })
        }
    }
}

impl EffectSelection<Mutation> for Mutate {
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
pub(super) enum OperationalFailure {
    Unavailable,
}

impl ClassifyError for OperationalFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

impl ClassifyError for Number {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}
