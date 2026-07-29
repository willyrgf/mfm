//! Pure effect-state contract for recoverable EVM transaction submission.

use mfm_ids::{ContentRef, StableId};
use mfm_program::{
    boundary_content_ref, decode_boundary, CanonicalCodec, CapabilityOperation, EvidenceVerdict,
    FactSet, NoBoundaryValue, NoContext, Settlement, State, StateExecution, StateFrame, UnitConfig,
    VerifiedTerminalEffectView,
};
use mfm_values::RetainedValueContract;

use crate::{
    EvmSubmitTransactionFailure, EvmSubmitTransactionInput, EvmSubmitTransactionRequest,
    EvmTransactionOutcome, EvmWalletAttemptResult, EVM_SUBMIT_TRANSACTION_OPERATION_ID,
};

/// Terminal executor outcome for a finalized successful EVM execution.
pub const EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME: &str = "mfm.evm.transaction_succeeded";
/// Terminal executor outcome for a finalized reverted EVM execution.
pub const EVM_WALLET_REVERTED_TERMINAL_OUTCOME: &str = "mfm.evm.transaction_reverted";

/// One recoverable effect state for the complete EVM wallet lifecycle.
pub struct SubmitEvmTransactionState;

impl State for SubmitEvmTransactionState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = EvmSubmitTransactionInput;
    type Output = EvmTransactionOutcome;
    type Failure = EvmSubmitTransactionFailure;
    type Request = EvmSubmitTransactionRequest;
    type Observation = NoBoundaryValue;
    type SafeDiagnostic = NoBoundaryValue;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        boundary_content_ref(
            crate::state::contract_schema_id("mfm.evm.state-contract")?,
            &crate::state::state_contract_canonical("submit_transaction")?,
        )
    }
}

fn submit_transaction_request(
    frame: StateFrame<'_, SubmitEvmTransactionState>,
) -> EvmSubmitTransactionRequest {
    EvmSubmitTransactionRequest::from_input(frame.input().value())
}

fn submit_transaction_settle(
    frame: StateFrame<'_, SubmitEvmTransactionState>,
    terminal: VerifiedTerminalEffectView<'_>,
) -> EvidenceVerdict<Settlement<SubmitEvmTransactionState>> {
    let fields = match terminal.evidence().fields() {
        Ok(fields) => fields,
        Err(_) => return EvidenceVerdict::InvalidEvidence,
    };
    let operation_id = match StableId::new(EVM_SUBMIT_TRANSACTION_OPERATION_ID) {
        Ok(operation_id) => operation_id,
        Err(_) => return EvidenceVerdict::InvalidEvidence,
    };
    if fields.external_operation_identity != operation_id {
        return EvidenceVerdict::InvalidEvidence;
    }
    let domain_bytes = match terminal.resolve(&fields.domain_evidence_ref) {
        Ok(bytes) => bytes,
        Err(_) => return EvidenceVerdict::InvalidEvidence,
    };
    let attempt = match decode_boundary::<EvmWalletAttemptResult>(&domain_bytes) {
        Ok(attempt) => attempt,
        Err(_) => return EvidenceVerdict::InvalidEvidence,
    };
    let evidence = match attempt.terminal_evidence() {
        Ok(Some(evidence)) => evidence,
        Ok(None) | Err(_) => return EvidenceVerdict::InvalidEvidence,
    };
    if evidence.request() != frame.input().value().request()
        || evidence
            .assurance_policy_ref()
            .to_content_ref()
            .ok()
            .as_ref()
            != Some(&fields.assurance_policy_ref)
    {
        return EvidenceVerdict::InvalidEvidence;
    }
    let outcome = match evidence.outcome() {
        Ok(outcome) => outcome,
        Err(_) => return EvidenceVerdict::InvalidEvidence,
    };
    let expected_terminal_outcome = match outcome {
        EvmTransactionOutcome::Succeeded { .. } => EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME,
        EvmTransactionOutcome::Reverted { .. } => EVM_WALLET_REVERTED_TERMINAL_OUTCOME,
    };
    if fields.terminal_outcome.as_str() != expected_terminal_outcome {
        return EvidenceVerdict::InvalidEvidence;
    }
    EvidenceVerdict::Settlement(Settlement::succeeded(outcome, FactSet::empty()))
}

/// Builds the one effect execution surface for EVM wallet submission.
#[allow(clippy::too_many_arguments)]
pub(crate) fn submit_transaction_execution(
    executor_contract_ref: ContentRef,
    binding_ref: ContentRef,
    request_contract: RetainedValueContract,
    ensure_result_contract: RetainedValueContract,
    terminal_evidence_contract: RetainedValueContract,
    domain_result_contract: RetainedValueContract,
) -> mfm_program::Result<StateExecution<SubmitEvmTransactionState>> {
    Ok(StateExecution::effect(
        CapabilityOperation::new(
            executor_contract_ref,
            StableId::new(EVM_SUBMIT_TRANSACTION_OPERATION_ID)
                .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))?,
            binding_ref,
        ),
        CanonicalCodec::mfm_value(request_contract)?,
        ensure_result_contract,
        terminal_evidence_contract,
        domain_result_contract,
        submit_transaction_request,
        submit_transaction_settle,
    ))
}
