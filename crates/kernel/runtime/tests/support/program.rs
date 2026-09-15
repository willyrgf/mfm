use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};

use mfm_capabilities::{CapabilityError, EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::{ContentRef, EffectId, SemanticTypeId, StableId};
use mfm_program::{
    CapabilityInjection, Classification, ClassifyError, EffectState, Identity, Never, NoParams,
    Occurrence, Operation, OperationExpansion, ProgramError, ProposedStateOutcome, PureState,
    ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::{
    canonicalize_mfm_value, InvocationDiagnostic, MfmValue as MfmValueTrait, SchemaAudit,
    SchemaDescriptor,
};
use serde::{Deserialize, Serialize};

pub(super) const PREPARATION_FAILURE_SENTINEL: u64 = u64::MAX;

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct Number {
    pub(super) value: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NumberAlias {
    value: u64,
}

impl MfmValueTrait for NumberAlias {
    fn schema_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        Number::schema_descriptor()
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        Number::semantic_id()
    }
}

pub(super) static ALTERNATE_DESCRIPTOR_AUDIT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MutableDescriptorNumber {
    value: u64,
}

impl MfmValueTrait for MutableDescriptorNumber {
    fn schema_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        let mut descriptor = Number::schema_descriptor()?;
        let rust_type = if ALTERNATE_DESCRIPTOR_AUDIT.load(Ordering::SeqCst) {
            "MutableDescriptorNumber::alternate"
        } else {
            "MutableDescriptorNumber"
        };
        descriptor.audit =
            SchemaAudit::__derive_generated("mfm-runtime-tests", rust_type, "test-only");
        Ok(descriptor)
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        Number::semantic_id()
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct FirstGenericValue {
    pub(super) first: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct SecondGenericValue {
    pub(super) second: String,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.test.runtime",
    name = "generic-state-value",
    version = "1",
    schema = "mfm.test.runtime-generic-state-value"
)]
pub(super) struct GenericStateValue<K> {
    pub(super) value: K,
}

pub(super) struct GenericState<K>(PhantomData<fn() -> K>);

impl<K: MfmValueTrait> State for GenericState<K> {
    type Input = GenericStateValue<K>;
    type Output = GenericStateValue<K>;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/generic-state@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl<K: MfmValueTrait> PureState for GenericState<K> {
    fn evaluate(
        input: Self::Input,
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic>
    {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}

pub(super) struct ConflictingGenericState<K>(pub(super) PhantomData<fn() -> K>);

impl<K: MfmValueTrait> State for ConflictingGenericState<K> {
    type Input = GenericStateValue<K>;
    type Output = GenericStateValue<K>;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        GenericState::<K>::state_id()
    }
}

impl<K: MfmValueTrait> PureState for ConflictingGenericState<K> {
    fn evaluate(
        input: Self::Input,
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic>
    {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}

pub(super) struct GenericProgram<K>(pub(super) PhantomData<fn() -> K>);

impl<K: MfmValueTrait> Operation for GenericProgram<K> {
    type Input = GenericStateValue<K>;
    type Output = GenericStateValue<K>;
    type Failure = Never;

    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<GenericState<K>, Identity<Never>>(NoParams, Occurrence::new())
    }
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

pub(super) struct PureProgram;

impl Operation for PureProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Never;

    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new())
    }
}

pub(super) struct EmptyProgram;

impl Operation for EmptyProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Never;

    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        _body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    pub(super) value: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct Evidence {
    pub(super) intent_value_ref: ContentRef,
    pub(super) value: u64,
    pub(super) accepted: bool,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub(super) route: u64,
}

pub(super) struct Observation;

impl ReadCapabilityContract for Observation {
    type OperationalError = OperationalFailure;
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/observation@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent_value_ref: &ContentRef,
        intent: &Self::Intent,
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

impl CapabilityInjection<Observe> for Observation {
    type FailureMap = Identity<Number>;
    fn failure_map_params(_: &Self::Setup) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = <Observe as State>::Failure;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}

pub(super) struct ReadProgram;

impl Operation for ReadProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.read::<Observe, Observation, Identity<Number>>(
            &Binding { route: 7 },
            NoParams,
            Occurrence::new(),
        )
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct Command {
    pub(super) value: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct EffectEvidence {
    pub(super) effect_id: EffectId,
    pub(super) value: u64,
    pub(super) accepted: bool,
}

pub(super) struct Mutation;

impl EffectCapabilityContract for Mutation {
    type OperationalError = OperationalFailure;
    type Command = Command;
    type Evidence = EffectEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/mutation@1").map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        effect_id: &EffectId,
        command: &Self::Command,
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
    type OperationalError = OperationalFailure;
    type Intent = Command;
    type Evidence = EffectEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Mutation::contract_id()
    }

    fn bind_evidence(
        _intent_value_ref: &ContentRef,
        intent: &Self::Intent,
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

impl CapabilityInjection<Mutate> for Mutation {
    type FailureMap = Identity<Number>;
    fn failure_map_params(_: &Self::Setup) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = <Mutate as State>::Failure;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}

pub(super) struct EffectProgram;

impl Operation for EffectProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.effect::<Mutate, Mutation, Identity<Number>>(
            &Binding { route: 8 },
            NoParams,
            Occurrence::new(),
        )
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
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
