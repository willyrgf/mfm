//! Durable keyed-executor orchestration for one qualified EVM wallet.

use std::collections::BTreeMap;

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
    verify_ensure_result, AccountSequenceAllocation, AccountSequencePolicy, AccountSequenceRequest,
    AllocationOutcome, DeliveryAttemptOutcome, EffectEntryView, ExecutorEnsureResultClaim,
    ExecutorError, ExecutorLedgerStore, ExecutorRetainedClosureClaim,
    ExecutorTerminalEvidenceClaim, FencingRef, KeyedExecutorLedger, ProofBasis,
    ReferenceTerminalProof, ResourcePolicyBinding, ReturnedOutcome, TerminalTombstone,
    VerifiedEnsureResult,
};
use mfm_ids::{AttemptId, ContentRef};
use mfm_program::decode_boundary;
use mfm_runtime::{AuthorizedEnsureAccess, RecoverableEffectExecutor};
use mfm_values::MfmValue;

use crate::{
    EvmWalletJsonRpcTarget, EvmWalletLiveError, EvmWalletRpcClient, EvmWalletSignerBinding,
    EvmWalletTargetEntryDescriptor,
};

/// Qualified durable wallet executor over one exact ledger, signer, and RPC target.
#[derive(Clone)]
pub struct EvmWalletExecutor<Store, Client>
where
    Store: ExecutorLedgerStore,
    Client: EvmWalletRpcClient,
{
    ledger: KeyedExecutorLedger<Store>,
    target: EvmWalletJsonRpcTarget<Client>,
    resource_policy_binding: ResourcePolicyBinding,
    generation_fence_ref: ContentRef,
}

impl<Store, Client> EvmWalletExecutor<Store, Client>
where
    Store: ExecutorLedgerStore,
    Client: EvmWalletRpcClient,
{
    /// Binds the high-level wallet to one verified durable executor deployment.
    pub fn new(
        ledger: KeyedExecutorLedger<Store>,
        client: Client,
        signer: EvmWalletSignerBinding,
        resource_policy_binding: ResourcePolicyBinding,
    ) -> mfm_executor::Result<Self> {
        let binding = ledger.exact_binding();
        let ownership = binding
            .resource_ownership()
            .ok_or(ExecutorError::ResourceOwnershipRequired)?;
        let generation_fence_ref = ownership
            .destination_fencing_authority_ref()
            .cloned()
            .ok_or(ExecutorError::DestinationGenerationFenced)?;
        if signer.durable_generation_ref() != binding.deployment().durable_ledger_generation_ref()
            || signer.fence_attestation_ref() != &generation_fence_ref
        {
            return Err(ExecutorError::LedgerGenerationMismatch);
        }
        let target = EvmWalletJsonRpcTarget::new(
            client,
            signer,
            binding.contract().safe_failure_contract_ref().clone(),
        );
        Ok(Self {
            ledger,
            target,
            resource_policy_binding,
            generation_fence_ref,
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
    ) -> mfm_executor::Result<VerifiedEnsureResult> {
        let request = committed.request();
        self.validate_request(request)?;
        let identity = committed.identity();
        let allocation = self.allocate_nonce(identity, request).await?;
        let mut view = self
            .ledger
            .effect_view(identity)
            .await?
            .ok_or(ExecutorError::EffectNotBound)?;
        if view.terminal_tombstone().is_some() {
            return self.terminal_return(view);
        }
        if let Some(terminal) = terminal_attempt(&view, request, self.exact_binding())? {
            view = self.append_terminal(identity, terminal).await?;
            return self.terminal_return(view);
        }

        let history = self
            .load_history(&view, request, allocation.sequence())
            .await?;
        let Some(plan) = history.next_plan(
            request,
            allocation.sequence(),
            self.exact_binding()
                .deployment()
                .durable_ledger_generation_ref(),
            &self.generation_fence_ref,
        )?
        else {
            return pending_return(view, self.exact_binding());
        };
        let expected_head = view.delivery_audit().head_ref()?;
        let view = match self
            .execute_plan(identity, request, &expected_head, plan)
            .await?
        {
            Some(view) => view,
            None => self
                .ledger
                .effect_view(identity)
                .await?
                .ok_or(ExecutorError::EffectNotBound)?,
        };
        if view.terminal_tombstone().is_some() {
            return self.terminal_return(view);
        }
        if let Some(terminal) = terminal_attempt(&view, request, self.exact_binding())? {
            let view = self.append_terminal(identity, terminal).await?;
            return self.terminal_return(view);
        }
        pending_return(view, self.exact_binding())
    }

    fn validate_request(&self, request: &EvmSubmitTransactionRequest) -> mfm_executor::Result<()> {
        let binding = self.exact_binding();
        let policy = request.policy();
        let wallet_domain_ref = policy
            .wallet_domain_ref()
            .to_content_ref()
            .map_err(|_| ExecutorError::TargetOperationMismatch)?;
        let nonce_attestation_ref = policy
            .initial_nonce_attestation_ref()
            .to_content_ref()
            .map_err(|_| ExecutorError::TargetOperationMismatch)?;
        if policy
            .tenant_scope_id()
            .map_err(|_| ExecutorError::TenantScopeMismatch)?
            != *binding.deployment().tenant_scope_id()
            || policy.evidence_bounds() != binding.contract().evidence_bounds()
            || binding.contract().resource_domain_requirement() != Some(&wallet_domain_ref)
            || binding
                .resource_ownership()
                .map(|ownership| ownership.external_resource_domain_ref())
                != Some(&wallet_domain_ref)
            || self.resource_policy_binding.policy_configuration_ref() != &nonce_attestation_ref
        {
            return Err(ExecutorError::TargetOperationMismatch);
        }
        Ok(())
    }

    async fn allocate_nonce(
        &self,
        identity: &mfm_executor::EffectIdentity,
        request: &EvmSubmitTransactionRequest,
    ) -> mfm_executor::Result<AccountSequenceAllocation> {
        let policy = request.policy();
        let wallet_domain_ref = policy
            .wallet_domain_ref()
            .to_content_ref()
            .map_err(|_| ExecutorError::TargetOperationMismatch)?;
        let resource_domain_preimage = format!(
            "{}:{}",
            wallet_domain_ref.schema_id().as_str(),
            wallet_domain_ref.content_digest().as_str()
        );
        let resource_request = AccountSequenceRequest::new(
            format!(
                "eip155-{}-{}",
                policy.chain_id(),
                policy.sender().trim_start_matches("0x")
            ),
            format!(
                "evm-wallet-domain-{}",
                sha256_digest_bytes(resource_domain_preimage.as_bytes())
            ),
        )
        .map_err(ExecutorError::ResourcePolicy)?;
        let sequence_policy = AccountSequencePolicy::new(
            self.resource_policy_binding.clone(),
            policy
                .initial_nonce()
                .map_err(|_| ExecutorError::TargetOperationMismatch)?,
            Some(FencingRef::from_reviewed(self.generation_fence_ref.clone())),
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
            != Some(&self.generation_fence_ref)
        {
            return Err(ExecutorError::ResourcePolicyNotRevalidated);
        }
        Ok(allocation)
    }

    async fn load_history(
        &self,
        view: &EffectEntryView,
        request: &EvmSubmitTransactionRequest,
        allocated_nonce: u64,
    ) -> mfm_executor::Result<WalletHistory> {
        let attempts = view
            .delivery_audit()
            .attempts()?
            .into_iter()
            .map(|attempt| (attempt.attempt_id().clone(), attempt.outcome().cloned()))
            .collect::<Vec<_>>();
        let mut history = WalletHistory::default();
        for (attempt_id, outcome) in attempts {
            let expected = history
                .next_plan(
                    request,
                    allocated_nonce,
                    self.exact_binding()
                        .deployment()
                        .durable_ledger_generation_ref(),
                    &self.generation_fence_ref,
                )?
                .ok_or(ExecutorError::TargetOperationMismatch)?;
            let value = self
                .ledger
                .target_operation(view.identity(), &attempt_id)
                .await?;
            let descriptor =
                EvmWalletTargetEntryDescriptor::strict_decode(&value).map_err(map_live_error)?;
            let candidate = descriptor.candidate(request).map_err(map_live_error)?;
            expected.validate_descriptor(&descriptor, &candidate, allocated_nonce)?;
            let returned = outcome
                .as_ref()
                .and_then(DeliveryAttemptOutcome::returned_outcome)
                .map(|returned| {
                    decode_attempt_result(returned).and_then(|result| {
                        expected.validate_result(&candidate, &result)?;
                        Ok((returned.clone(), result))
                    })
                })
                .transpose()?;
            history.push(WalletAttempt {
                descriptor,
                candidate,
                returned,
            })?;
        }
        Ok(history)
    }

    async fn execute_plan(
        &self,
        identity: &mfm_executor::EffectIdentity,
        request: &EvmSubmitTransactionRequest,
        expected_head: &mfm_executor::DeliveryAuditFrontierRef,
        mut plan: WalletPlan,
    ) -> mfm_executor::Result<Option<EffectEntryView>> {
        let receipt = match plan.kind {
            WalletPlanKind::Broadcast => {
                let prepared = self
                    .target
                    .prepare_broadcast(request, plan.allocated_nonce, plan.fee_ordinal())
                    .await
                    .map_err(map_live_error)?;
                if plan
                    .candidate()
                    .is_some_and(|candidate| candidate != prepared.candidate())
                {
                    return Err(ExecutorError::TargetOperationMismatch);
                }
                let descriptor = plan.descriptor(request, prepared.candidate())?;
                let Some(authority) = self
                    .ledger
                    .try_authorize_target(
                        identity,
                        expected_head,
                        descriptor.canonical().clone(),
                        Some(&self.resource_policy_binding),
                    )
                    .await?
                else {
                    return Ok(None);
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
                        Some(&self.resource_policy_binding),
                    )
                    .await?
                else {
                    return Ok(None);
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
                        return Err(ExecutorError::TargetOperationMismatch);
                    }
                }
            }
        };
        self.ledger.observe_target(receipt).await.map(Some)
    }

    async fn append_terminal(
        &self,
        identity: &mfm_executor::EffectIdentity,
        terminal: TerminalAttempt,
    ) -> mfm_executor::Result<EffectEntryView> {
        let proof = ReferenceTerminalProof::new(
            terminal.attempt_id,
            terminal.returned,
            terminal.observation_ref,
        )?;
        let tombstone =
            TerminalTombstone::new(EVM_SUBMIT_TRANSACTION_OPERATION_ID, terminal.outcome, proof)?;
        match self
            .ledger
            .append_terminal_tombstone(identity, tombstone)
            .await
        {
            Ok(view) => Ok(view),
            Err(ExecutorError::TerminalTombstoneConflict) => self
                .ledger
                .effect_view(identity)
                .await?
                .ok_or(ExecutorError::EffectNotBound),
            Err(error) => Err(error),
        }
    }

    fn terminal_return(&self, view: EffectEntryView) -> mfm_executor::Result<VerifiedEnsureResult> {
        let (tombstone_ref, tombstone) = view
            .terminal_tombstone()
            .cloned()
            .ok_or(ExecutorError::TerminalEvidenceMissing)?;
        let proof = tombstone.terminal_proof().clone();
        let audit = view.delivery_audit().clone();
        let domain_evidence_ref = proof.returned_outcome().safe_result_ref().clone();
        let attempt = decode_attempt_result(proof.returned_outcome())?;
        let terminal = attempt
            .terminal_evidence()
            .map_err(|_| ExecutorError::TerminalProofMismatch)?
            .ok_or(ExecutorError::TerminalEvidenceMissing)?;
        let claim = ExecutorTerminalEvidenceClaim::new(
            view.identity().clone(),
            audit.head_ref()?,
            tombstone_ref,
            tombstone.external_operation_identity(),
            tombstone.terminal_outcome(),
            terminal
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
}

impl<Store, Client> RecoverableEffectExecutor<EvmSubmitTransactionRequest>
    for EvmWalletExecutor<Store, Client>
where
    Store: ExecutorLedgerStore,
    Client: EvmWalletRpcClient,
{
    fn ensure<'a>(
        &'a self,
        access: AuthorizedEnsureAccess<EvmSubmitTransactionRequest>,
    ) -> mfm_executor::ExecutorFuture<'a, mfm_executor::Result<VerifiedEnsureResult>> {
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
}

struct WalletAttempt {
    descriptor: EvmWalletTargetEntryDescriptor,
    candidate: EvmWalletTransactionCandidate,
    returned: Option<(ReturnedOutcome, EvmWalletAttemptResult)>,
}

impl WalletHistory {
    fn push(&mut self, attempt: WalletAttempt) -> mfm_executor::Result<()> {
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

    fn next_plan(
        &self,
        request: &EvmSubmitTransactionRequest,
        allocated_nonce: u64,
        executor_generation_ref: &ContentRef,
        generation_fence_ref: &ContentRef,
    ) -> mfm_executor::Result<Option<WalletPlan>> {
        if self
            .attempts
            .iter()
            .any(|attempt| terminal_result(attempt).is_some())
        {
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
                                executor_generation_ref.clone(),
                            ),
                            mfm_evm::EvmWalletReference::from_content_ref(
                                generation_fence_ref.clone(),
                            ),
                            request.policy().assurance_policy_ref().clone(),
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

struct TerminalAttempt {
    attempt_id: AttemptId,
    returned: ReturnedOutcome,
    observation_ref: ContentRef,
    outcome: &'static str,
}

fn terminal_attempt(
    view: &EffectEntryView,
    request: &EvmSubmitTransactionRequest,
    binding: &mfm_executor::VerifiedExecutorBinding,
) -> mfm_executor::Result<Option<TerminalAttempt>> {
    for attempt in view.delivery_audit().attempts()?.into_iter().rev() {
        let Some(returned) = attempt
            .outcome()
            .and_then(DeliveryAttemptOutcome::returned_outcome)
        else {
            continue;
        };
        let result = decode_attempt_result(returned)?;
        let Some(evidence) = result
            .terminal_evidence()
            .map_err(|_| ExecutorError::TerminalProofMismatch)?
        else {
            continue;
        };
        if evidence.request() != request
            || evidence
                .executor_generation_ref()
                .to_content_ref()
                .ok()
                .as_ref()
                != Some(binding.deployment().durable_ledger_generation_ref())
            || evidence
                .generation_fence_ref()
                .to_content_ref()
                .ok()
                .as_ref()
                != binding
                    .resource_ownership()
                    .and_then(|ownership| ownership.destination_fencing_authority_ref())
        {
            return Err(ExecutorError::TerminalProofMismatch);
        }
        let outcome = match evidence
            .outcome()
            .map_err(|_| ExecutorError::TerminalProofMismatch)?
        {
            EvmTransactionOutcome::Succeeded { .. } => EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME,
            EvmTransactionOutcome::Reverted { .. } => EVM_WALLET_REVERTED_TERMINAL_OUTCOME,
        };
        return Ok(Some(TerminalAttempt {
            attempt_id: attempt.attempt_id().clone(),
            returned: returned.clone(),
            observation_ref: attempt
                .returned_observation_ref()
                .cloned()
                .ok_or(ExecutorError::TerminalProofMismatch)?,
            outcome,
        }));
    }
    Ok(None)
}

fn terminal_result(attempt: &WalletAttempt) -> Option<&EvmWalletTerminalEvidence> {
    attempt
        .returned
        .as_ref()
        .and_then(|(_, result)| result.terminal_evidence().ok().flatten())
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

fn pending_return(
    view: EffectEntryView,
    binding: &mfm_executor::VerifiedExecutorBinding,
) -> mfm_executor::Result<VerifiedEnsureResult> {
    let audit = view.delivery_audit().clone();
    let head = audit.head_ref()?;
    let retained = ExecutorRetainedClosureClaim::from_delivery_audit(&audit, binding)?;
    verify_ensure_result(
        view.identity().clone(),
        binding,
        ExecutorEnsureResultClaim::pending(head),
        retained,
    )
}

fn map_live_error(error: EvmWalletLiveError) -> ExecutorError {
    match error {
        EvmWalletLiveError::InvalidContract => ExecutorError::TargetOperationMismatch,
        EvmWalletLiveError::ResultEncoding => ExecutorError::CanonicalEncoding,
        EvmWalletLiveError::SignerUnavailable => ExecutorError::DurableBackendUnavailable,
    }
}
