use super::*;

/// Context-bound state that prepares and submits contract deployment transactions.
pub struct ContextBoundDeployContractState {
    action: DeployAction,
}

impl StateSpec for ContextBoundDeployContractState {
    type Config = DeployAction;
    type Context = EvmContractContext;
    type Input = ();
    type Output = DeployedContractInstance;
    type Effect = ApplySideEffect;
    type Caps = ContractMutationCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("context_deploy")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("context_deploy")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.context_deploy"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        produces_context(contract_instance_resource_kind(), deployed_contract_stage())
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            action: config.into_inner(),
        })
    }
}

impl SideEffectState for ContextBoundDeployContractState {
    type Intent = ContextContractDeployIntent;
    type IdempotencyInput = ContractTransactionIdempotency;
    type Submission = ContractTransactionSubmissions;
    type Receipt = ContractDeployReceipt;
    type Confirmation = ContractDeployConfirmation;
    type SubmitFuture<'a> = future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(
        &self,
        _input: &Self::Input,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Intent> {
        Ok(ContextContractDeployIntent {
            intent_version: 1,
            transaction: context_transaction_intent_from_deploy_action(context, &self.action, None),
            constructor_args_len: self.action.constructor_args().len() as u64,
            contract_profile_id: context
                .value()
                .contract_profile
                .profile_id
                .as_str()
                .to_owned(),
        })
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::IdempotencyInput> {
        idempotency_from_intent(intent)
    }

    fn submit<'a>(
        &'a self,
        _intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }

    fn output_from_receipt(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        receipt: &Self::Receipt,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        deployed_instance_from_receipt(receipt.contract_address.as_str(), &receipt.receipt, context)
    }

    fn output_from_confirmation(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        deployed_instance_from_receipt(
            confirmation.contract_address.as_str(),
            &confirmation.receipt,
            context,
        )
    }
}

/// Context-bound state that prepares and submits contract configuration transactions.
pub struct ContextBoundConfigureContractState {
    action: ConfigureAction,
}

impl StateSpec for ContextBoundConfigureContractState {
    type Config = ConfigureAction;
    type Context = EvmContractContext;
    type Input = ContextConfigureContractInput;
    type Output = ConfiguredContractInstance;
    type Effect = ApplySideEffect;
    type Caps = ContractMutationCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("context_configure")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("context_configure")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.context_configure"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn input_context_contract() -> mfm_program::Result<mfm_program::StateInputContextContractSpec> {
        requires_context(
            contract_instance_resource_kind(),
            deployed_contract_stage(),
            vec![descriptor_id_for_state::<ContextBoundDeployContractState>()?],
        )
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        produces_context(
            contract_instance_resource_kind(),
            configured_contract_stage(),
        )
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            action: config.into_inner(),
        })
    }
}

impl SideEffectState for ContextBoundConfigureContractState {
    type Intent = ContextContractConfigureIntent;
    type IdempotencyInput = ContractTransactionIdempotency;
    type Submission = ContractTransactionSubmissions;
    type Receipt = ContextContractConfigureReceipt;
    type Confirmation = ContextContractConfigureConfirmation;
    type SubmitFuture<'a> = future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(
        &self,
        input: &Self::Input,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Intent> {
        let transactions = self
            .action
            .calls()
            .iter()
            .map(|call| {
                context_transaction_intent_from_configure_call(
                    context,
                    &self.action,
                    &input.deployed,
                    call,
                )
            })
            .collect();
        Ok(ContextContractConfigureIntent {
            intent_version: 1,
            deployed: input.deployed.clone(),
            transactions,
        })
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::IdempotencyInput> {
        idempotency_from_intent(intent)
    }

    fn submit<'a>(
        &'a self,
        _intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }

    fn output_from_receipt(
        &self,
        input: &Self::Input,
        _intent: &Self::Intent,
        receipt: &Self::Receipt,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        let expected_anchor = configured_contract_anchor_from_receipts(input, &receipt.receipts)?;
        if receipt.anchor != expected_anchor {
            return Err(StateError::Message(
                "configuration receipt anchor does not match certified receipt order".to_owned(),
            ));
        }
        configured_instance_from_evidence(input, &receipt.anchor, context)
    }

    fn output_from_confirmation(
        &self,
        input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        let expected_anchor =
            configured_contract_anchor_from_receipts(input, &confirmation.receipts)?;
        if confirmation.anchor != expected_anchor {
            return Err(StateError::Message(
                "configuration confirmation anchor does not match certified receipt order"
                    .to_owned(),
            ));
        }
        configured_instance_from_evidence(input, &confirmation.anchor, context)
    }
}
