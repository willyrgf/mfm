use mfm_capabilities::{AdapterError, EffectCapabilityContract};
use mfm_ids::{ContentRef, DigestBytes, EffectId, EntryPointId, RunId, StableId};
use mfm_journal::{JournalHistory, JournalRecord, PendingDecision, StopCode};
use mfm_program::*;
use mfm_program_derive::MfmValue;
use mfm_runtime::{
    EffectAdapterOutcome, InvocationFailure, RunViewState, Runtime, RuntimeAssemblyBuilder,
    RuntimeError, SizeResource,
};
use mfm_store::{MemoryStore, Store};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Number {
    value: u64,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum Cause {
    Timeout { deadline_ms: u64 },
}
impl ClassifyError for Cause {
    fn classify(&self) -> Classification {
        Classification::OutcomeUnknown
    }
}
struct Submit;
impl EffectCapabilityContract for Submit {
    type Command = Number;
    type Evidence = Number;
    type OperationalError = Cause;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.audited-submit@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        _: &EffectId,
        command: &Number,
        evidence: &Number,
    ) -> mfm_capabilities::Result<()> {
        if command.value == evidence.value {
            Ok(())
        } else {
            Err(mfm_capabilities::CapabilityError::EvidenceBinding)
        }
    }
}
struct Execute;
impl State for Execute {
    type Input = Number;
    type Output = Number;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.audited-execute@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl EffectState<Submit> for Execute {
    type AdapterContext = Number;
    fn adapter_context(
        input: &Number,
        _: &Number,
        _: &Cause,
    ) -> std::result::Result<Number, StateExecutionError> {
        Ok(Number { value: input.value })
    }
    fn prepare(input: &Number) -> std::result::Result<Number, PreparationError> {
        Ok(Number { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &Number,
    ) -> std::result::Result<ProposedStateOutcome<Number, Never>, StateExecutionError> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl CapabilityInjection<Execute> for Submit {
    type Setup = Number;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = Never;
    type FailureMap = Identity<Never>;
    fn failure_map_params(_: &Number) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &Number) -> mfm_program::Result<ContentRef> {
        mfm_values::canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}
struct RetryUnknown;
impl Handler for RetryUnknown {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.retry-unknown@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        incident: &IncidentSummary,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        assert_eq!(incident.classification, Classification::OutcomeUnknown);
        Ok(RecoveryRequest::RetryState)
    }
}
struct Flow;
impl Operation for Flow {
    type Input = Number;
    type Output = Number;
    type Failure = Never;
    fn validate_input(&self, _: &Number) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut OperationExpansion<Number, Number, Never>,
    ) -> mfm_program::Result<()> {
        scope.handler(HandlerBinding::new::<RetryUnknown>(NoParams)?)?;
        scope.allowances(RecoveryAllowances::new(1, 0))?;
        scope.effect::<Execute, Submit, Identity<Never>>(
            &Number { value: 1 },
            NoParams,
            Occurrence::new(),
            EffectBounds::new(65536, 65536, 2, 65536)?,
        )
    }
}

struct StopFlow;
impl Operation for StopFlow {
    type Input = Number;
    type Output = Number;
    type Failure = Never;
    fn validate_input(&self, _: &Number) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut OperationExpansion<Number, Number, Never>,
    ) -> mfm_program::Result<()> {
        scope.handler(HandlerBinding::new::<StandardRecovery>(NoParams)?)?;
        scope.effect::<Execute, Submit, Identity<Never>>(
            &Number { value: 1 },
            NoParams,
            Occurrence::new(),
            EffectBounds::new(65536, 65536, 2, 65536)?,
        )
    }
}

#[allow(dead_code)]
#[path = "support/scripted_store.rs"]
mod scripted_store;

#[path = "pending_failure/audit.rs"]
mod audit;
#[path = "pending_failure/phase.rs"]
mod phase;
#[path = "pending_failure/protocol.rs"]
mod protocol;
