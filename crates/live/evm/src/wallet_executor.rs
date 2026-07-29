//! Durable keyed-executor orchestration for one qualified EVM wallet.

use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_canonical::sha256_digest_bytes;
use mfm_evm::{
    EvmSubmitTransactionRequest, EvmTransactionOutcome, EvmWalletAttemptResult,
    EvmWalletTerminalEvidence, EvmWalletTransactionCandidate, EVM_SUBMIT_TRANSACTION_OPERATION_ID,
    EVM_WALLET_BROADCAST_OPERATION_ID, EVM_WALLET_FINALIZED_HEAD_OPERATION_ID,
    EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID, EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID,
    EVM_WALLET_REVERTED_TERMINAL_OUTCOME, EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME,
    EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID,
};
use mfm_executor::{
    reference_safe_failure, verify_ensure_result, AccountSequenceAllocation, AccountSequencePolicy,
    AccountSequenceRequest, AllocationOutcome, BoundaryStage, DeliveryAttemptOutcome,
    EffectEntryView, EffectExecutorOutcome, ExecutorEnsureResultClaim, ExecutorError,
    ExecutorEvidenceRecord, ExecutorLedgerStore, ExecutorRetainedClosureClaim,
    ExecutorTerminalEvidenceClaim, FailureClass, FencingRef, KeyedExecutorLedger, ProofBasis,
    ReferenceFailureCode, ReferenceTerminalProof, ReturnedOutcome, TerminalTombstone,
    VerifiedEnsureResult,
};
use mfm_ids::{AttemptId, ContentRef};
use mfm_program::decode_boundary;
use mfm_runtime::{AuthorizedEnsureAccess, RecoverableEffectExecutor};
use mfm_signing::GenerationGuardedDeterministicSigningProviderBinder;
use mfm_values::MfmValue;

use crate::{
    wallet_rpc::{EvmWalletJsonRpcTarget, EvmWalletTargetEntryDescriptor},
    EvmWalletLiveError, EvmWalletRequestQualification,
};

/// Qualified durable wallet executor over one exact ledger, signer, and RPC target.
#[derive(Clone)]
pub struct EvmWalletExecutor<Store>
where
    Store: ExecutorLedgerStore,
{
    ledger: KeyedExecutorLedger<Store>,
    target: EvmWalletJsonRpcTarget,
    qualification: Arc<EvmWalletRequestQualification>,
}

enum WalletDriveError {
    SignerUnavailable,
    Executor(ExecutorError),
}

impl From<ExecutorError> for WalletDriveError {
    fn from(error: ExecutorError) -> Self {
        Self::Executor(error)
    }
}

impl<Store> EvmWalletExecutor<Store>
where
    Store: ExecutorLedgerStore,
{
    /// Binds the high-level wallet to one verified durable executor deployment.
    pub fn new(
        ledger: KeyedExecutorLedger<Store>,
        signer: GenerationGuardedDeterministicSigningProviderBinder,
        qualification: Arc<EvmWalletRequestQualification>,
    ) -> mfm_executor::Result<Self> {
        if qualification.executor_binding() != ledger.exact_binding() {
            return Err(ExecutorError::LedgerGenerationMismatch);
        }
        let target = EvmWalletJsonRpcTarget::new(signer, Arc::clone(&qualification))
            .map_err(map_live_error)?;
        Ok(Self {
            ledger,
            target,
            qualification,
        })
    }

    /// Returns the exact verified executor binding.
    pub const fn exact_binding(&self) -> &mfm_executor::VerifiedExecutorBinding {
        self.ledger.exact_binding()
    }

    /// Drives at most one newly authorized target exchange.
    pub async fn drive(
        &self,
        committed: &mfm_executor::CommittedEffectRequest<EvmSubmitTransactionRequest>,
    ) -> mfm_executor::Result<EffectExecutorOutcome> {
        match self.drive_result(committed).await {
            Ok(result) => Ok(EffectExecutorOutcome::returned(result)),
            Err(WalletDriveError::SignerUnavailable) => {
                let failure = reference_safe_failure(
                    self.exact_binding()
                        .contract()
                        .safe_failure_contract_ref()
                        .clone(),
                    ReferenceFailureCode::DestinationUnavailable,
                    FailureClass::Transport,
                    BoundaryStage::BeforeBoundaryEntry,
                )?;
                EffectExecutorOutcome::did_not_enter(failure)
            }
            Err(WalletDriveError::Executor(error)) => Err(error),
        }
    }

    async fn drive_result(
        &self,
        committed: &mfm_executor::CommittedEffectRequest<EvmSubmitTransactionRequest>,
    ) -> Result<VerifiedEnsureResult, WalletDriveError> {
        let request = committed.request();
        self.qualification
            .verify_request(request)
            .map_err(map_live_error)?;
        let identity = committed.identity();
        let allocation = self.allocate_nonce(identity).await?;
        let mut verified = self
            .load_verified_history(identity, request, allocation.sequence())
            .await?;
        if verified.view.terminal_tombstone().is_some() {
            return Ok(self.terminal_return(verified)?);
        }
        if let Some(terminal) = verified.history.terminal.as_ref() {
            self.append_terminal(identity, terminal).await?;
            verified = self
                .load_verified_history(identity, request, allocation.sequence())
                .await?;
            return Ok(self.terminal_return(verified)?);
        }

        let Some(plan) = verified.history.next_plan(
            request,
            allocation.sequence(),
            self.qualification.as_ref(),
        )?
        else {
            return Ok(self.pending_return(verified)?);
        };
        let expected_head = verified.view.delivery_audit().head_ref()?;
        self.execute_plan(identity, request, &expected_head, plan)
            .await?;
        verified = self
            .load_verified_history(identity, request, allocation.sequence())
            .await?;
        if verified.view.terminal_tombstone().is_some() {
            return Ok(self.terminal_return(verified)?);
        }
        if let Some(terminal) = verified.history.terminal.as_ref() {
            self.append_terminal(identity, terminal).await?;
            verified = self
                .load_verified_history(identity, request, allocation.sequence())
                .await?;
            return Ok(self.terminal_return(verified)?);
        }
        Ok(self.pending_return(verified)?)
    }

    async fn allocate_nonce(
        &self,
        identity: &mfm_executor::EffectIdentity,
    ) -> mfm_executor::Result<AccountSequenceAllocation> {
        let wallet_domain_ref = self.qualification.wallet_domain_ref();
        let resource_domain_preimage = format!(
            "{}:{}",
            wallet_domain_ref.schema_id().as_str(),
            wallet_domain_ref.content_digest().as_str()
        );
        let resource_request = AccountSequenceRequest::new(
            format!(
                "eip155-{}-{}",
                self.qualification.chain_id(),
                format!("{:#x}", self.qualification.sender()).trim_start_matches("0x")
            ),
            format!(
                "evm-wallet-domain-{}",
                sha256_digest_bytes(resource_domain_preimage.as_bytes())
            ),
        )
        .map_err(ExecutorError::ResourcePolicy)?;
        let sequence_policy = AccountSequencePolicy::new(
            self.qualification.resource_policy_binding().clone(),
            self.qualification
                .initial_nonce_descriptor()
                .initial_nonce()
                .map_err(|_| ExecutorError::TargetOperationMismatch)?,
            Some(FencingRef::from_reviewed(
                self.qualification.generation_fence_ref().clone(),
            )),
        );
        let resource_key =
            mfm_executor::TypedResourcePolicy::resource_key(&sequence_policy, &resource_request)
                .map_err(ExecutorError::ResourcePolicy)?;
        let expected_head = self.ledger.resource_head(&resource_key).await?;
        let outcome = self
            .ledger
            .try_bind_and_allocate(
                identity,
                expected_head.as_ref(),
                &sequence_policy,
                &resource_request,
            )
            .await?;
        let (allocation, evidence) = match outcome {
            AllocationOutcome::Allocated {
                allocation,
                evidence,
                ..
            }
            | AllocationOutcome::Existing {
                allocation,
                evidence,
                ..
            } => (allocation, evidence),
        };
        if evidence.fencing_ref().map(FencingRef::as_content_ref)
            != Some(self.qualification.generation_fence_ref())
        {
            return Err(ExecutorError::ResourcePolicyNotRevalidated);
        }
        Ok(allocation)
    }

    async fn load_verified_history(
        &self,
        identity: &mfm_executor::EffectIdentity,
        request: &EvmSubmitTransactionRequest,
        allocated_nonce: u64,
    ) -> mfm_executor::Result<VerifiedWalletHistory> {
        let view = self
            .ledger
            .effect_view(identity)
            .await?
            .ok_or(ExecutorError::EffectNotBound)?;
        let mut history = WalletHistory::default();
        for frontier in view.delivery_audit().frontiers() {
            for record in frontier.appended_records() {
                match record {
                    ExecutorEvidenceRecord::DeliveryAttemptAuthorized { attempt_id, .. } => {
                        let plan = history
                            .next_plan(request, allocated_nonce, self.qualification.as_ref())?
                            .ok_or(ExecutorError::TargetOperationMismatch)?;
                        let value = self.ledger.target_operation(identity, attempt_id).await?;
                        let descriptor = EvmWalletTargetEntryDescriptor::strict_decode(&value)
                            .map_err(map_live_error)?;
                        let candidate = descriptor.candidate(request).map_err(map_live_error)?;
                        plan.validate_descriptor(&descriptor, &candidate, allocated_nonce)?;
                        history.push(WalletAttempt {
                            attempt_id: attempt_id.clone(),
                            plan,
                            descriptor,
                            candidate,
                            observed: false,
                            returned: None,
                        })?;
                    }
                    ExecutorEvidenceRecord::DeliveryAttemptObserved {
                        attempt_id,
                        outcome,
                    } => {
                        let observation_ref = record
                            .observed_content_ref()?
                            .ok_or(ExecutorError::InvalidDeliveryObservation)?;
                        history.observe(attempt_id, outcome, observation_ref)?;
                    }
                    ExecutorEvidenceRecord::TerminalTombstone(tombstone) => {
                        validate_tombstone_relation(tombstone, history.terminal.as_ref())?;
                    }
                    ExecutorEvidenceRecord::EffectBound { .. }
                    | ExecutorEvidenceRecord::ResourceAllocated(_) => {}
                }
            }
        }
        Ok(VerifiedWalletHistory { view, history })
    }

    async fn execute_plan(
        &self,
        identity: &mfm_executor::EffectIdentity,
        request: &EvmSubmitTransactionRequest,
        expected_head: &mfm_executor::DeliveryAuditFrontierRef,
        mut plan: WalletPlan,
    ) -> Result<(), WalletDriveError> {
        let receipt = match plan.kind {
            WalletPlanKind::Broadcast => {
                let prepared = self
                    .target
                    .prepare_broadcast(request, plan.allocated_nonce, plan.fee_ordinal())
                    .await
                    .map_err(|error| match error {
                        EvmWalletLiveError::SignerUnavailable => {
                            WalletDriveError::SignerUnavailable
                        }
                        error => WalletDriveError::Executor(map_live_error(error)),
                    })?;
                if plan
                    .candidate()
                    .is_some_and(|candidate| candidate != prepared.candidate())
                {
                    return Err(ExecutorError::TargetOperationMismatch.into());
                }
                let descriptor = plan.descriptor(request, prepared.candidate())?;
                let Some(authority) = self
                    .ledger
                    .try_authorize_target(
                        identity,
                        expected_head,
                        descriptor.canonical().clone(),
                        Some(self.qualification.resource_policy_binding()),
                    )
                    .await?
                else {
                    return Ok(());
                };
                let returned = self
                    .target
                    .broadcast(authority, request, prepared)
                    .await
                    .map_err(map_live_error)?;
                returned.into_parts().0
            }
            WalletPlanKind::TransactionLookup
            | WalletPlanKind::ReceiptLookup
            | WalletPlanKind::FinalizedHead
            | WalletPlanKind::CanonicalInclusion => {
                let terminal_evidence = plan.terminal_evidence.take();
                let candidate = plan
                    .candidate()
                    .ok_or(ExecutorError::TargetOperationMismatch)?;
                let descriptor = plan.descriptor(request, candidate)?;
                let Some(authority) = self
                    .ledger
                    .try_authorize_target(
                        identity,
                        expected_head,
                        descriptor.canonical().clone(),
                        Some(self.qualification.resource_policy_binding()),
                    )
                    .await?
                else {
                    return Ok(());
                };
                match plan.kind {
                    WalletPlanKind::TransactionLookup => self
                        .target
                        .transaction_lookup(authority, request, candidate)
                        .await
                        .map_err(map_live_error)?,
                    WalletPlanKind::ReceiptLookup => self
                        .target
                        .receipt_lookup(authority, request, candidate)
                        .await
                        .map_err(map_live_error)?,
                    WalletPlanKind::FinalizedHead => self
                        .target
                        .finalized_head(authority, request, candidate)
                        .await
                        .map_err(map_live_error)?,
                    WalletPlanKind::CanonicalInclusion => self
                        .target
                        .canonical_inclusion(
                            authority,
                            request,
                            candidate,
                            plan.inclusion_number
                                .ok_or(ExecutorError::TargetOperationMismatch)?,
                            terminal_evidence,
                        )
                        .await
                        .map_err(map_live_error)?,
                    WalletPlanKind::Broadcast => {
                        return Err(ExecutorError::TargetOperationMismatch.into());
                    }
                }
            }
        };
        self.ledger.observe_target(receipt).await?;
        Ok(())
    }

    async fn append_terminal(
        &self,
        identity: &mfm_executor::EffectIdentity,
        terminal: &TerminalAttempt,
    ) -> mfm_executor::Result<()> {
        let proof = ReferenceTerminalProof::new(
            terminal.attempt_id.clone(),
            terminal.returned.clone(),
            terminal.observation_ref.clone(),
        )?;
        let tombstone =
            TerminalTombstone::new(EVM_SUBMIT_TRANSACTION_OPERATION_ID, terminal.outcome, proof)?;
        match self
            .ledger
            .append_terminal_tombstone(identity, tombstone)
            .await
        {
            Ok(_) | Err(ExecutorError::TerminalTombstoneConflict) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn terminal_return(
        &self,
        verified: VerifiedWalletHistory,
    ) -> mfm_executor::Result<VerifiedEnsureResult> {
        let VerifiedWalletHistory { view, history } = verified;
        let terminal = history
            .terminal
            .ok_or(ExecutorError::TerminalEvidenceMissing)?;
        let (tombstone_ref, tombstone) = view
            .terminal_tombstone()
            .cloned()
            .ok_or(ExecutorError::TerminalEvidenceMissing)?;
        let audit = view.delivery_audit().clone();
        let domain_evidence_ref = terminal.returned.safe_result_ref().clone();
        let claim = ExecutorTerminalEvidenceClaim::new(
            view.identity().clone(),
            audit.head_ref()?,
            tombstone_ref,
            tombstone.external_operation_identity(),
            tombstone.terminal_outcome(),
            terminal
                .evidence
                .assurance_policy_ref()
                .to_content_ref()
                .map_err(|_| ExecutorError::TerminalProofMismatch)?,
            ProofBasis::ExecutorAttestation {
                evidence_authority_ref: self
                    .exact_binding()
                    .deployment()
                    .evidence_authority_ref()
                    .clone(),
            },
            domain_evidence_ref,
        )?;
        let retained =
            ExecutorRetainedClosureClaim::from_delivery_audit(&audit, self.exact_binding())?;
        verify_ensure_result(
            view.identity().clone(),
            self.exact_binding(),
            ExecutorEnsureResultClaim::terminal(claim),
            retained,
        )
    }

    fn pending_return(
        &self,
        verified: VerifiedWalletHistory,
    ) -> mfm_executor::Result<VerifiedEnsureResult> {
        if verified.history.terminal.is_some() || verified.view.terminal_tombstone().is_some() {
            return Err(ExecutorError::TerminalProofMismatch);
        }
        let audit = verified.view.delivery_audit().clone();
        let head = audit.head_ref()?;
        let retained =
            ExecutorRetainedClosureClaim::from_delivery_audit(&audit, self.exact_binding())?;
        verify_ensure_result(
            verified.view.identity().clone(),
            self.exact_binding(),
            ExecutorEnsureResultClaim::pending(head),
            retained,
        )
    }
}

impl<Store> RecoverableEffectExecutor<EvmSubmitTransactionRequest> for EvmWalletExecutor<Store>
where
    Store: ExecutorLedgerStore,
{
    fn ensure<'a>(
        &'a self,
        access: AuthorizedEnsureAccess<EvmSubmitTransactionRequest>,
    ) -> mfm_executor::ExecutorFuture<'a, mfm_executor::Result<EffectExecutorOutcome>> {
        Box::pin(async move { self.drive(access.committed_request()).await })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalletPlanKind {
    Broadcast,
    TransactionLookup,
    ReceiptLookup,
    FinalizedHead,
    CanonicalInclusion,
}

struct WalletPlan {
    kind: WalletPlanKind,
    allocated_nonce: u64,
    fee_ordinal: u16,
    candidate: Option<EvmWalletTransactionCandidate>,
    inclusion_number: Option<alloy_primitives::U256>,
    terminal_evidence: Option<EvmWalletTerminalEvidence>,
}

impl WalletPlan {
    fn simple(
        kind: WalletPlanKind,
        allocated_nonce: u64,
        fee_ordinal: u16,
        candidate: Option<EvmWalletTransactionCandidate>,
    ) -> Self {
        Self {
            kind,
            allocated_nonce,
            fee_ordinal,
            candidate,
            inclusion_number: None,
            terminal_evidence: None,
        }
    }

    fn fee_ordinal(&self) -> u16 {
        self.fee_ordinal
    }

    fn candidate(&self) -> Option<&EvmWalletTransactionCandidate> {
        self.candidate.as_ref()
    }

    fn operation_id(&self) -> &'static str {
        match self.kind {
            WalletPlanKind::Broadcast => EVM_WALLET_BROADCAST_OPERATION_ID,
            WalletPlanKind::TransactionLookup => EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID,
            WalletPlanKind::ReceiptLookup => EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID,
            WalletPlanKind::FinalizedHead => EVM_WALLET_FINALIZED_HEAD_OPERATION_ID,
            WalletPlanKind::CanonicalInclusion => EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID,
        }
    }

    fn descriptor(
        &self,
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> mfm_executor::Result<EvmWalletTargetEntryDescriptor> {
        let descriptor = match self.kind {
            WalletPlanKind::Broadcast => {
                EvmWalletTargetEntryDescriptor::broadcast(request, candidate)
            }
            WalletPlanKind::TransactionLookup => {
                EvmWalletTargetEntryDescriptor::transaction_lookup(request, candidate)
            }
            WalletPlanKind::ReceiptLookup => {
                EvmWalletTargetEntryDescriptor::receipt_lookup(request, candidate)
            }
            WalletPlanKind::FinalizedHead => {
                EvmWalletTargetEntryDescriptor::finalized_head(request, candidate)
            }
            WalletPlanKind::CanonicalInclusion => {
                EvmWalletTargetEntryDescriptor::canonical_inclusion(
                    request,
                    candidate,
                    self.inclusion_number
                        .ok_or(ExecutorError::TargetOperationMismatch)?,
                )
            }
        };
        descriptor.map_err(map_live_error)
    }

    fn validate_descriptor(
        &self,
        descriptor: &EvmWalletTargetEntryDescriptor,
        candidate: &EvmWalletTransactionCandidate,
        allocated_nonce: u64,
    ) -> mfm_executor::Result<()> {
        if descriptor.operation_id() != self.operation_id()
            || descriptor.allocated_nonce().map_err(map_live_error)? != allocated_nonce
            || descriptor.fee_ordinal() != self.fee_ordinal
            || self
                .candidate
                .as_ref()
                .is_some_and(|expected| expected != candidate)
        {
            return Err(ExecutorError::TargetOperationMismatch);
        }
        if self.kind == WalletPlanKind::CanonicalInclusion
            && descriptor.inclusion_number().map_err(map_live_error)? != self.inclusion_number
        {
            return Err(ExecutorError::TargetOperationMismatch);
        }
        Ok(())
    }

    fn validate_result(
        &self,
        candidate: &EvmWalletTransactionCandidate,
        result: &EvmWalletAttemptResult,
    ) -> mfm_executor::Result<()> {
        result
            .validate_for_candidate(candidate)
            .map_err(|_| ExecutorError::TargetOperationMismatch)?;
        let correct_variant = matches!(
            (self.kind, result),
            (
                WalletPlanKind::Broadcast,
                EvmWalletAttemptResult::Broadcast { .. }
            ) | (
                WalletPlanKind::TransactionLookup,
                EvmWalletAttemptResult::TransactionLookup { .. }
            ) | (
                WalletPlanKind::ReceiptLookup,
                EvmWalletAttemptResult::ReceiptLookup { .. }
            ) | (
                WalletPlanKind::FinalizedHead,
                EvmWalletAttemptResult::FinalizedHead { .. }
            ) | (
                WalletPlanKind::CanonicalInclusion,
                EvmWalletAttemptResult::CanonicalInclusion { .. }
            )
        );
        if !correct_variant {
            return Err(ExecutorError::TargetOperationMismatch);
        }
        if let EvmWalletAttemptResult::CanonicalInclusion { block, terminal } = result {
            let expected_terminal = block.as_ref().and_then(|block| {
                self.terminal_evidence
                    .as_ref()
                    .filter(|evidence| evidence.inclusion_block() == block)
            });
            if terminal.as_ref() != expected_terminal {
                return Err(ExecutorError::TargetOperationMismatch);
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct WalletHistory {
    attempts: Vec<WalletAttempt>,
    candidates: BTreeMap<u16, EvmWalletTransactionCandidate>,
    terminal: Option<TerminalAttempt>,
}

struct WalletAttempt {
    attempt_id: AttemptId,
    plan: WalletPlan,
    descriptor: EvmWalletTargetEntryDescriptor,
    candidate: EvmWalletTransactionCandidate,
    observed: bool,
    returned: Option<(ReturnedOutcome, EvmWalletAttemptResult)>,
}

impl WalletHistory {
    fn push(&mut self, attempt: WalletAttempt) -> mfm_executor::Result<()> {
        if self
            .attempts
            .iter()
            .any(|existing| existing.attempt_id == attempt.attempt_id)
        {
            return Err(ExecutorError::AttemptIdentityMismatch);
        }
        match self.candidates.entry(attempt.candidate.fee_ordinal()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(attempt.candidate.clone());
            }
            std::collections::btree_map::Entry::Occupied(entry)
                if entry.get() == &attempt.candidate => {}
            std::collections::btree_map::Entry::Occupied(_) => {
                return Err(ExecutorError::TargetOperationMismatch);
            }
        }
        self.attempts.push(attempt);
        Ok(())
    }

    fn observe(
        &mut self,
        attempt_id: &AttemptId,
        outcome: &DeliveryAttemptOutcome,
        observation_ref: ContentRef,
    ) -> mfm_executor::Result<()> {
        let terminal = {
            let attempt = self
                .attempts
                .iter_mut()
                .find(|attempt| &attempt.attempt_id == attempt_id)
                .ok_or(ExecutorError::InvalidDeliveryObservation)?;
            if attempt.observed {
                return Err(ExecutorError::InvalidDeliveryObservation);
            }
            attempt.observed = true;
            let Some(returned) = outcome.returned_outcome() else {
                return Ok(());
            };
            let result = decode_attempt_result(returned)?;
            attempt.plan.validate_result(&attempt.candidate, &result)?;
            let terminal = result
                .terminal_evidence()
                .map_err(|_| ExecutorError::TargetOperationMismatch)?
                .cloned()
                .map(|evidence| {
                    let outcome = terminal_outcome(&evidence)?;
                    Ok(TerminalAttempt {
                        attempt_id: attempt_id.clone(),
                        returned: returned.clone(),
                        observation_ref,
                        outcome,
                        evidence,
                    })
                })
                .transpose()?;
            attempt.returned = Some((returned.clone(), result));
            terminal
        };
        if self.terminal.is_none() {
            self.terminal = terminal;
        }
        Ok(())
    }

    fn next_plan(
        &self,
        request: &EvmSubmitTransactionRequest,
        allocated_nonce: u64,
        qualification: &EvmWalletRequestQualification,
    ) -> mfm_executor::Result<Option<WalletPlan>> {
        if self.terminal.is_some() {
            return Ok(None);
        }
        let convergence = request.policy().convergence();
        let finalized_attempts = self.count_operation(EVM_WALLET_FINALIZED_HEAD_OPERATION_ID);
        let inclusion_attempts = self.count_operation(EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID);
        for fee_index in 0..request.policy().replacement().fee_candidates().len() {
            let fee_ordinal =
                u16::try_from(fee_index).map_err(|_| ExecutorError::EvidenceBoundsExhausted)?;
            let candidate = self.candidates.get(&fee_ordinal).cloned();
            if let Some(candidate) = candidate.as_ref() {
                if let Some((transaction, receipt, finalized_head)) =
                    self.terminal_inputs(candidate)?
                {
                    if inclusion_attempts < usize::from(convergence.canonical_inclusion_lookups()) {
                        let attempt_result_refs = self
                            .attempts
                            .iter()
                            .filter_map(|attempt| {
                                attempt.returned.as_ref().map(|(returned, _)| {
                                    mfm_evm::EvmWalletReference::from_content_ref(
                                        returned.safe_result_ref().clone(),
                                    )
                                })
                            })
                            .collect();
                        let candidate_lineage = (0..=fee_ordinal)
                            .map(|ordinal| {
                                self.candidates
                                    .get(&ordinal)
                                    .cloned()
                                    .ok_or(ExecutorError::TargetOperationMismatch)
                            })
                            .collect::<mfm_executor::Result<Vec<_>>>()?;
                        let terminal = EvmWalletTerminalEvidence::new(
                            request.clone(),
                            attempt_result_refs,
                            candidate_lineage,
                            candidate.clone(),
                            transaction,
                            receipt.clone(),
                            finalized_head,
                            receipt.block().clone(),
                            mfm_evm::EvmWalletReference::from_content_ref(
                                qualification
                                    .executor_binding()
                                    .deployment()
                                    .durable_ledger_generation_ref()
                                    .clone(),
                            ),
                            mfm_evm::EvmWalletReference::from_content_ref(
                                qualification.generation_fence_ref().clone(),
                            ),
                            qualification.assurance_policy_ref().clone(),
                        )
                        .map_err(|_| ExecutorError::TargetOperationMismatch)?;
                        return Ok(Some(WalletPlan {
                            kind: WalletPlanKind::CanonicalInclusion,
                            allocated_nonce,
                            fee_ordinal,
                            candidate: Some(candidate.clone()),
                            inclusion_number: Some(
                                receipt
                                    .block()
                                    .number_quantity()
                                    .map_err(|_| ExecutorError::TargetOperationMismatch)?,
                            ),
                            terminal_evidence: Some(terminal),
                        }));
                    }
                } else if self.candidate_evidence(candidate)?.is_some()
                    && finalized_attempts < usize::from(convergence.finalized_head_lookups())
                {
                    return Ok(Some(WalletPlan::simple(
                        WalletPlanKind::FinalizedHead,
                        allocated_nonce,
                        fee_ordinal,
                        Some(candidate.clone()),
                    )));
                }
            }

            let counts = CandidateAttemptCounts::from_history(self, fee_ordinal);
            let rounds = *[
                convergence.broadcasts_per_candidate(),
                convergence.transaction_lookups_per_candidate(),
                convergence.receipt_lookups_per_candidate(),
            ]
            .iter()
            .max()
            .ok_or(ExecutorError::TargetOperationMismatch)?;
            let mut expected = CandidateAttemptCounts::default();
            for round in 0..rounds {
                for (enabled, kind) in [
                    (
                        round < convergence.broadcasts_per_candidate(),
                        WalletPlanKind::Broadcast,
                    ),
                    (
                        round < convergence.transaction_lookups_per_candidate(),
                        WalletPlanKind::TransactionLookup,
                    ),
                    (
                        round < convergence.receipt_lookups_per_candidate(),
                        WalletPlanKind::ReceiptLookup,
                    ),
                ] {
                    if !enabled {
                        continue;
                    }
                    if counts.for_kind(kind) == expected.for_kind(kind) {
                        if kind != WalletPlanKind::Broadcast && candidate.is_none() {
                            return Err(ExecutorError::TargetOperationMismatch);
                        }
                        return Ok(Some(WalletPlan::simple(
                            kind,
                            allocated_nonce,
                            fee_ordinal,
                            candidate.clone(),
                        )));
                    }
                    expected.increment(kind);
                }
            }
        }
        Ok(None)
    }

    fn count_operation(&self, operation_id: &str) -> usize {
        self.attempts
            .iter()
            .filter(|attempt| attempt.descriptor.operation_id() == operation_id)
            .count()
    }

    fn candidate_evidence(
        &self,
        candidate: &EvmWalletTransactionCandidate,
    ) -> mfm_executor::Result<
        Option<(
            mfm_evm::EvmWalletObservedTransaction,
            mfm_evm::EvmWalletReceipt,
        )>,
    > {
        let start = self.active_start(candidate);
        let transactions = self.attempts[start..]
            .iter()
            .filter(|attempt| &attempt.candidate == candidate)
            .filter_map(
                |attempt| match attempt.returned.as_ref().map(|(_, value)| value) {
                    Some(EvmWalletAttemptResult::TransactionLookup {
                        transaction: Some(transaction),
                        ..
                    }) => Some(transaction.clone()),
                    _ => None,
                },
            )
            .collect::<Vec<_>>();
        let receipts = self.attempts[start..]
            .iter()
            .filter(|attempt| &attempt.candidate == candidate)
            .filter_map(
                |attempt| match attempt.returned.as_ref().map(|(_, value)| value) {
                    Some(EvmWalletAttemptResult::ReceiptLookup {
                        receipt: Some(receipt),
                        ..
                    }) => Some(receipt.clone()),
                    _ => None,
                },
            )
            .collect::<Vec<_>>();
        for receipt in receipts.iter().rev() {
            for transaction in transactions.iter().rev() {
                if receipt
                    .matches_candidate(candidate, transaction)
                    .map_err(|_| ExecutorError::TargetOperationMismatch)?
                {
                    return Ok(Some((transaction.clone(), receipt.clone())));
                }
            }
        }
        Ok(None)
    }

    fn terminal_inputs(
        &self,
        candidate: &EvmWalletTransactionCandidate,
    ) -> mfm_executor::Result<
        Option<(
            mfm_evm::EvmWalletObservedTransaction,
            mfm_evm::EvmWalletReceipt,
            mfm_evm::EvmBlockAnchor,
        )>,
    > {
        let Some((transaction, receipt)) = self.candidate_evidence(candidate)? else {
            return Ok(None);
        };
        let start = self.active_start(candidate);
        let receipt_number = receipt
            .block()
            .number_quantity()
            .map_err(|_| ExecutorError::TargetOperationMismatch)?;
        let finalized = self.attempts[start..]
            .iter()
            .filter(|attempt| &attempt.candidate == candidate)
            .filter_map(
                |attempt| match attempt.returned.as_ref().map(|(_, value)| value) {
                    Some(EvmWalletAttemptResult::FinalizedHead { block }) => Some(block),
                    _ => None,
                },
            )
            .rev()
            .find(|block| {
                block
                    .number_quantity()
                    .is_ok_and(|number| number >= receipt_number)
            })
            .cloned();
        Ok(finalized.map(|finalized| (transaction, receipt, finalized)))
    }

    fn active_start(&self, candidate: &EvmWalletTransactionCandidate) -> usize {
        self.attempts
            .iter()
            .enumerate()
            .filter(|(_, attempt)| &attempt.candidate == candidate)
            .filter(|(_, attempt)| {
                matches!(
                    attempt.returned.as_ref().map(|(_, result)| result),
                    Some(EvmWalletAttemptResult::CanonicalInclusion { terminal: None, .. })
                )
            })
            .map(|(index, _)| index + 1)
            .next_back()
            .unwrap_or(0)
    }
}

#[derive(Default)]
struct CandidateAttemptCounts {
    broadcasts: usize,
    transaction_lookups: usize,
    receipt_lookups: usize,
}

impl CandidateAttemptCounts {
    fn from_history(history: &WalletHistory, fee_ordinal: u16) -> Self {
        let mut counts = Self::default();
        for attempt in history
            .attempts
            .iter()
            .filter(|attempt| attempt.candidate.fee_ordinal() == fee_ordinal)
        {
            match attempt.descriptor.operation_id() {
                EVM_WALLET_BROADCAST_OPERATION_ID => counts.broadcasts += 1,
                EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID => counts.transaction_lookups += 1,
                EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID => counts.receipt_lookups += 1,
                EVM_WALLET_FINALIZED_HEAD_OPERATION_ID
                | EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID => {}
                _ => {}
            }
        }
        counts
    }

    fn for_kind(&self, kind: WalletPlanKind) -> usize {
        match kind {
            WalletPlanKind::Broadcast => self.broadcasts,
            WalletPlanKind::TransactionLookup => self.transaction_lookups,
            WalletPlanKind::ReceiptLookup => self.receipt_lookups,
            WalletPlanKind::FinalizedHead | WalletPlanKind::CanonicalInclusion => 0,
        }
    }

    fn increment(&mut self, kind: WalletPlanKind) {
        match kind {
            WalletPlanKind::Broadcast => self.broadcasts += 1,
            WalletPlanKind::TransactionLookup => self.transaction_lookups += 1,
            WalletPlanKind::ReceiptLookup => self.receipt_lookups += 1,
            WalletPlanKind::FinalizedHead | WalletPlanKind::CanonicalInclusion => {}
        }
    }
}

struct VerifiedWalletHistory {
    view: EffectEntryView,
    history: WalletHistory,
}

struct TerminalAttempt {
    attempt_id: AttemptId,
    returned: ReturnedOutcome,
    observation_ref: ContentRef,
    outcome: &'static str,
    evidence: EvmWalletTerminalEvidence,
}

fn validate_tombstone_relation(
    tombstone: &TerminalTombstone,
    terminal: Option<&TerminalAttempt>,
) -> mfm_executor::Result<()> {
    match terminal {
        None => Err(ExecutorError::TerminalEvidenceMissing),
        Some(terminal) => {
            let proof = tombstone.terminal_proof();
            if tombstone.external_operation_identity() != EVM_SUBMIT_TRANSACTION_OPERATION_ID
                || tombstone.terminal_outcome() != terminal.outcome
                || proof.attempt_id() != &terminal.attempt_id
                || proof.returned_outcome() != &terminal.returned
                || proof.returned_observation_ref() != &terminal.observation_ref
            {
                return Err(ExecutorError::TerminalProofMismatch);
            }
            Ok(())
        }
    }
}

fn terminal_outcome(evidence: &EvmWalletTerminalEvidence) -> mfm_executor::Result<&'static str> {
    match evidence
        .outcome()
        .map_err(|_| ExecutorError::TerminalProofMismatch)?
    {
        EvmTransactionOutcome::Succeeded { .. } => Ok(EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME),
        EvmTransactionOutcome::Reverted { .. } => Ok(EVM_WALLET_REVERTED_TERMINAL_OUTCOME),
    }
}

fn decode_attempt_result(
    returned: &ReturnedOutcome,
) -> mfm_executor::Result<EvmWalletAttemptResult> {
    if returned.safe_result().schema_id()
        != &EvmWalletAttemptResult::schema_id().map_err(|_| ExecutorError::CanonicalEncoding)?
    {
        return Err(ExecutorError::RetainedValueContractMismatch);
    }
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_canonical_json_slice(
        returned.safe_result().as_bytes(),
    )
    .map_err(|_| ExecutorError::CanonicalEncoding)?;
    decode_boundary(&canonical).map_err(|_| ExecutorError::CanonicalEncoding)
}

fn map_live_error(error: EvmWalletLiveError) -> ExecutorError {
    match error {
        EvmWalletLiveError::InvalidContract => ExecutorError::TargetOperationMismatch,
        EvmWalletLiveError::ResultEncoding => ExecutorError::CanonicalEncoding,
        EvmWalletLiveError::SignerUnavailable => ExecutorError::DurableBackendUnavailable,
    }
}
