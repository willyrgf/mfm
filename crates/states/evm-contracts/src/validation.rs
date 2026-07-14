use super::*;

/// Context-bound state that validates a configured contract through read-only EVM capabilities.
pub struct ContextBoundValidateContractState {
    action: ValidateAction,
}

impl ContextBoundValidateContractState {
    /// Builds the deterministic read request an adapter must execute.
    pub fn read_request(
        &self,
        input: &ContextValidateContractInput,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
    ) -> StateResult<ContextContractValidationReadRequest> {
        Ok(ContextContractValidationReadRequest {
            request_version: 1,
            context_ref: ContextRefValue::from(context.context_ref().clone()),
            configured_input_digest: digest_for_config(&input.configured)?,
            configured_instance: ConfiguredContractInstanceRef::from_configured(&input.configured),
            network_id: context.value().network.network_id.as_str().to_owned(),
            expected_chain_id: context.value().network.expected_chain_id(),
            read_assertions: self.action.read_assertions().to_vec(),
            event_assertions: self.action.event_assertions().to_vec(),
        })
    }

    /// Projects a context-bound validation report from an adapter-provided read response.
    pub fn report_from_response(
        &self,
        input: &ContextValidateContractInput,
        response: ContractValidationReadResponse,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
    ) -> StateResult<ContextBoundValidationReport> {
        let configured_input_digest = digest_for_config(&input.configured)?;
        if response.context_ref.as_context_ref() != context.context_ref()
            || response.configured_input_digest != configured_input_digest
            || response.resource_stage != ContractLifecycleStage::Configured
        {
            return Err(StateError::Message(
                "validation read response context does not match certified invocation".to_owned(),
            ));
        }
        require_validation_read_results_canonical_passed(&response.configuration_read_results)?;
        require_validation_event_results_canonical_passed(&response.configuration_event_results)?;
        require_validation_read_results_canonical_passed(&response.read_results)?;
        require_validation_event_results_canonical_passed(&response.event_results)?;

        let valid = response.observed_chain_id == context.value().network.expected_chain_id()
            && response
                .configuration_read_results
                .iter()
                .all(validation_read_result_passes)
            && response
                .configuration_event_results
                .iter()
                .all(validation_event_result_passes)
            && response
                .read_results
                .iter()
                .all(validation_read_result_passes)
            && response
                .event_results
                .iter()
                .all(validation_event_result_passes);

        Ok(ContextBoundValidationReport {
            report_version: 1,
            context_ref: ContextRefValue::from(context.context_ref().clone()),
            configured_instance: ConfiguredContractInstanceRef::from_configured(&input.configured),
            observed_chain_id: response.observed_chain_id,
            configuration_read_results: response.configuration_read_results,
            configuration_event_results: response.configuration_event_results,
            read_results: response.read_results,
            event_results: response.event_results,
            validation_read_evidence: response.validation_read_evidence,
            validation_event_evidence: response.validation_event_evidence,
            evidence_refs: response.evidence_refs,
            valid,
        })
    }
}

impl StateSpec for ContextBoundValidateContractState {
    type Config = ValidateAction;
    type Context = EvmContractContext;
    type Input = ContextValidateContractInput;
    type Output = ContextBoundValidationReport;
    type Effect = ReadExternal;
    type Caps = ContractValidationReadCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("context_validate")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("context_validate")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.context_validate"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn input_context_contract() -> mfm_program::Result<mfm_program::StateInputContextContractSpec> {
        requires_context(
            contract_instance_resource_kind(),
            configured_contract_stage(),
            vec![
                descriptor_id_for_state::<ContextBoundConfigureContractState>()?,
                descriptor_id_for_state::<ImportConfiguredContractState>()?,
            ],
        )
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        produces_context(validation_report_resource_kind(), validation_report_stage())
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            action: config.into_inner(),
        })
    }
}

impl ReadState for ContextBoundValidateContractState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }
}
