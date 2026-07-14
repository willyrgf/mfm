use super::*;

pub(super) async fn run_context_verify(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let phase = {
        let submit_node = side_effect_verify_submit_node(&ctx)?;
        context_mutation_phase_for_submit_node(submit_node)?
    };
    match phase {
        ContractMutationPhase::Deploy => {
            let callbacks = ContractVerifyCallbacks::<ContextDeployMutationPlan> {
                factory,
                _phase: PhantomData,
            };
            SideEffectVerifyDriver::drive(ctx, &callbacks).await
        }
        ContractMutationPhase::Configure => {
            let callbacks = ContractVerifyCallbacks::<ContextConfigureMutationPlan> {
                factory,
                _phase: PhantomData,
            };
            SideEffectVerifyDriver::drive(ctx, &callbacks).await
        }
    }
}

struct ContractVerifyCallbacks<'a, P> {
    factory: &'a dyn EvmContractRuntimeFactory,
    _phase: PhantomData<P>,
}

impl<P> ContractVerifyCallbacks<'_, P>
where
    P: ContractVerifyPhase,
{
    async fn load_plan(
        &self,
        ctx: &ErasedRunCtx<'_>,
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
    ) -> mfm_runtime::Result<P> {
        let submit_context = ctx.invocation_context_for_node(submit_node)?;
        P::load_plan(
            submit_node,
            submit_inputs,
            &submit_context,
            self.factory.artifacts(),
        )
        .await
    }

    async fn load_plan_and_runtime(
        &self,
        ctx: &ErasedRunCtx<'_>,
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
    ) -> mfm_runtime::Result<(P, EvmContractRuntime)> {
        let plan = self.load_plan(ctx, submit_node, submit_inputs).await?;
        let binding = runtime_evm_network_binding(plan.network_id(), plan.expected_chain_id())?;
        let runtime = self.factory.runtime_for(binding)?;
        Ok((plan, runtime))
    }

    async fn load_prepared_invocation(
        &self,
        submit_node: &spec::NodeSpec,
        projection: &store::SideEffectArtifactProjection,
    ) -> mfm_runtime::Result<PreparedContractInvocation> {
        load_prepared_invocation_for_node(
            projection,
            events::ArtifactRole::PreparedInvocation,
            self.factory.artifacts(),
            &submit_node.node_id,
        )
        .await
    }

    async fn load_reconstructed_prepared_invocation(
        &self,
        ctx: &ErasedRunCtx<'_>,
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
        projection: &store::SideEffectArtifactProjection,
    ) -> mfm_runtime::Result<(P, EvmContractRuntime, PreparedContractMutation)> {
        let stored_prepared = self
            .load_prepared_invocation(submit_node, projection)
            .await?;
        let (plan, runtime) = self
            .load_plan_and_runtime(ctx, submit_node, submit_inputs)
            .await?;
        let prepared = plan.reconstruct_prepared_invocation(&runtime, &stored_prepared)?;
        Ok((plan, runtime, prepared))
    }
}

impl<P> SideEffectVerifyCallbacks for ContractVerifyCallbacks<'_, P>
where
    P: ContractVerifyPhase,
{
    type Submission = ContractTransactionSubmissions;
    type Receipt = P::Receipt;
    type Confirmation = P::Confirmation;
    type Output = <P as ContractMutationPlanOps>::Output;
    type NotSubmittedProof = ContractNotSubmittedProof;
    type AmbiguityEvidence = ContractTransactionSubmissions;

    fn recover_unknown_submission<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        prepared_invocation: Option<&'a store::SideEffectArtifactProjection>,
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectUnknownSubmissionDecision<
            Self::Submission,
            Self::NotSubmittedProof,
            Self::AmbiguityEvidence,
        >,
    > {
        Box::pin(async move {
            let prepared_projection = prepared_invocation
                .ok_or_else(|| missing_side_effect_artifact("prepared invocation"))?;
            let (_plan, runtime, prepared) = self
                .load_reconstructed_prepared_invocation(
                    ctx,
                    submit_node,
                    submit_inputs,
                    prepared_projection,
                )
                .await?;
            recover_unknown_prepared_contract_submission(&runtime, &prepared).await
        })
    }

    fn read_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        Box::pin(async {
            let prepared_projection = projected_prepared_artifact_for_submit(ctx, submit_node)?;
            let (plan, runtime, prepared) = self
                .load_reconstructed_prepared_invocation(
                    ctx,
                    submit_node,
                    submit_inputs,
                    &prepared_projection,
                )
                .await?;
            let (submissions, _) =
                load_side_effect_artifact_for_node::<ContractTransactionSubmissions>(
                    submission,
                    events::ArtifactRole::Submission,
                    &submit_node.node_id,
                    self.factory.artifacts(),
                )
                .await?;
            let receipts =
                read_receipts_with_poll(&runtime, prepared.evidence(), &submissions).await?;
            Ok(SideEffectObservedEvidence::new(
                plan.receipt_from_observed(prepared.evidence(), receipts)?,
                contract_side_effect_replay_evidence()?,
            ))
        })
    }

    fn build_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        Box::pin(async {
            let (_plan, runtime) = self
                .load_plan_and_runtime(ctx, submit_node, submit_inputs)
                .await?;
            let required_depth = finalized_depth_for_submit_node(submit_node)?;
            let (receipt, receipt_evidence) = load_side_effect_artifact_for_node::<P::Receipt>(
                receipt,
                events::ArtifactRole::Receipt,
                &ctx.node().node_id,
                self.factory.artifacts(),
            )
            .await?;
            let receipt_evidence = CapabilityArtifactEvidenceRef::from(receipt_evidence);
            let receipt = P::receipt_with_evidence(receipt, &receipt_evidence);
            let confirmations = verified_finality_confirmations(
                &runtime,
                P::receipt_transactions(&receipt),
                required_depth,
            )
            .await?;
            Ok(SideEffectObservedEvidence::new(
                P::confirmation_from_receipt(receipt, confirmations),
                contract_side_effect_replay_evidence()?,
            ))
        })
    }

    fn map_receipt_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async {
            let plan = self.load_plan(ctx, submit_node, submit_inputs).await?;
            let (receipt, _) = load_side_effect_artifact_for_node::<P::Receipt>(
                receipt,
                events::ArtifactRole::Receipt,
                &ctx.node().node_id,
                self.factory.artifacts(),
            )
            .await?;
            plan.output_from_receipt(&receipt)
        })
    }

    fn map_confirmation_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async {
            let plan = self.load_plan(ctx, submit_node, submit_inputs).await?;
            let (confirmation, _) = load_side_effect_artifact_for_node::<P::Confirmation>(
                confirmation,
                events::ArtifactRole::Confirmation,
                &ctx.node().node_id,
                self.factory.artifacts(),
            )
            .await?;
            plan.output_from_confirmation(&confirmation)
        })
    }
}

trait ContractVerifyPhase: ContractMutationPlanOps + Sized + Send + Sync + 'static {
    type Receipt: MfmValue + Send + Sync + 'static;
    type Confirmation: MfmValue + Send + Sync + 'static;

    fn receipt_from_observed(
        &self,
        prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt>;

    fn receipt_with_evidence(
        receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt;

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt];

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation;

    fn output_from_receipt(
        &self,
        receipt: &Self::Receipt,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output>;

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output>;
}

impl ContractVerifyPhase for ContextDeployMutationPlan {
    type Receipt = ContractDeployReceipt;
    type Confirmation = ContractDeployConfirmation;

    fn receipt_from_observed(
        &self,
        prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt> {
        Ok(ContractDeployReceipt {
            receipt_version: 1,
            context_ref: prepared.context_ref.clone(),
            evm_network_context_ref: prepared.evm_network_context_ref.clone(),
            resource_stage: prepared.resource_stage,
            contract_address: deploy_contract_address_from_prepared(prepared)?,
            receipt: single_receipt(receipts)?,
        })
    }

    fn receipt_with_evidence(
        receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt {
        deploy_receipt_with_evidence(receipt, evidence)
    }

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt] {
        std::slice::from_ref(&receipt.receipt)
    }

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation {
        ContractDeployConfirmation {
            confirmation_version: 1,
            context_ref: receipt.context_ref,
            evm_network_context_ref: receipt.evm_network_context_ref,
            resource_stage: receipt.resource_stage,
            confirmations,
            contract_address: receipt.contract_address,
            receipt: receipt.receipt,
        }
    }

    fn output_from_receipt(
        &self,
        receipt: &Self::Receipt,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_receipt(&(), &self.intent, receipt, &self.context)
            .map_err(runtime_state_error)
    }

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_confirmation(&(), &self.intent, confirmation, &self.context)
            .map_err(runtime_state_error)
    }
}

impl ContractVerifyPhase for ContextConfigureMutationPlan {
    type Receipt = ContextContractConfigureReceipt;
    type Confirmation = ContextContractConfigureConfirmation;

    fn receipt_from_observed(
        &self,
        _prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt> {
        Ok(ContextContractConfigureReceipt {
            receipt_version: 1,
            context_ref: _prepared.context_ref.clone(),
            evm_network_context_ref: _prepared.evm_network_context_ref.clone(),
            resource_stage: _prepared.resource_stage,
            configure_node: LifecycleNodeIdRef::from(self.node_id.clone()),
            configured_block_number: receipts.iter().map(|receipt| receipt.block_number).max(),
            receipts,
            call_evidence_refs: Vec::new(),
            confirmation_evidence_refs: Vec::new(),
        })
    }

    fn receipt_with_evidence(
        mut receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt {
        let evidence = lifecycle_evidence_ref(evidence);
        for transaction_receipt in &mut receipt.receipts {
            transaction_receipt.receipt_evidence = Some(evidence.clone());
        }
        receipt
    }

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt] {
        &receipt.receipts
    }

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation {
        ContextContractConfigureConfirmation {
            confirmation_version: 1,
            context_ref: receipt.context_ref,
            evm_network_context_ref: receipt.evm_network_context_ref,
            resource_stage: receipt.resource_stage,
            confirmations,
            configure_node: receipt.configure_node,
            receipts: receipt.receipts,
            call_evidence_refs: receipt.call_evidence_refs,
            confirmation_evidence_refs: receipt.confirmation_evidence_refs,
            configured_block_number: receipt.configured_block_number,
        }
    }

    fn output_from_receipt(
        &self,
        receipt: &Self::Receipt,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_receipt(&self.input, &self.intent, receipt, &self.context)
            .map_err(runtime_state_error)
    }

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_confirmation(&self.input, &self.intent, confirmation, &self.context)
            .map_err(runtime_state_error)
    }
}
