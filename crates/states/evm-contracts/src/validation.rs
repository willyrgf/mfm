use super::*;
use alloy_primitives::keccak256;
use mfm_evm_contract_model::{
    EvmCodeHash, ValidationCodeIdentityReport, ValidationCodeIdentitySelector,
    ValidationSourceEvidence,
};

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
        let code_identity = context
            .value()
            .contract_profile
            .deployed_code_hash
            .clone()
            .map(|expected_code_hash| ValidationCodeIdentityRequest {
                expected_code_hash,
                selector: ValidationCodeIdentitySelector {
                    address: input.configured.address.clone(),
                    block_number: input.configured.anchor.block_number,
                    block_hash: input.configured.anchor.block_hash.clone(),
                    require_canonical: true,
                },
            });
        Ok(ContextContractValidationReadRequest {
            request_version: 1,
            context_ref: ContextRefValue::from(context.context_ref().clone()),
            configured_input_digest: digest_for_config(&input.configured)?,
            configured_instance: ConfiguredContractInstanceRef::from_configured(&input.configured),
            network_id: context.value().network.network_id.as_str().to_owned(),
            expected_chain_id: context.value().network.expected_chain_id(),
            code_identity,
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
            || response.evm_network_context_ref != evm_network_context_ref_for_context(context)?
            || response.resource_stage != ContractLifecycleStage::Configured
            || response.observed_chain_id != context.value().network.expected_chain_id()
        {
            return Err(StateError::Message(
                "validation read response context does not match certified invocation".to_owned(),
            ));
        }
        require_validation_evidence_matches_context(
            context,
            &response.read_results,
            &response.validation_read_evidence,
            &response.event_results,
            &response.validation_event_evidence,
        )?;
        require_validation_results_match_action(
            &self.action,
            &response.read_results,
            &response.event_results,
        )?;
        require_validation_read_results_canonical_passed(&response.read_results)?;
        require_validation_event_results_canonical_passed(&response.event_results)?;

        let request = self.read_request(input, context)?;
        let code_identity = match (
            &request.code_identity,
            response.code_identity_evidence.as_ref(),
        ) {
            (None, None) => None,
            (None, Some(_)) => {
                return Err(StateError::Message(
                    "validation response retained unexpected code identity evidence".to_owned(),
                ));
            }
            (Some(_), None) => {
                return Err(StateError::Message(
                    "validation response is missing required code identity evidence".to_owned(),
                ));
            }
            (Some(request), Some(evidence)) => {
                Some(validate_code_identity_evidence(context, request, evidence)?)
            }
        };

        let valid = response.observed_chain_id == context.value().network.expected_chain_id()
            && response
                .read_results
                .iter()
                .all(validation_read_result_passes)
            && response
                .event_results
                .iter()
                .all(validation_event_result_passes)
            && code_identity_matches_expected(
                request.code_identity.as_ref(),
                code_identity.as_ref(),
            );

        Ok(ContextBoundValidationReport {
            report_version: 1,
            context_ref: ContextRefValue::from(context.context_ref().clone()),
            configured_instance: ConfiguredContractInstanceRef::from_configured(&input.configured),
            observed_chain_id: response.observed_chain_id,
            read_results: response.read_results,
            event_results: response.event_results,
            validation_read_evidence: response.validation_read_evidence,
            validation_event_evidence: response.validation_event_evidence,
            code_identity,
            valid,
        })
    }
}

fn evm_network_context_ref_for_context(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> StateResult<String> {
    let json = serde_json::to_string(&context.value().network).map_err(|error| {
        StateError::Message(format!("network context serialization failed: {error}"))
    })?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.content_digest().to_string())
        .map_err(|error| {
            StateError::Message(format!("network context canonicalization failed: {error}"))
        })
}

fn require_validation_results_match_action(
    action: &ValidateAction,
    read_results: &[ValidationReadResult],
    event_results: &[ValidationEventResult],
) -> StateResult<()> {
    if read_results.len() != action.read_assertions().len()
        || event_results.len() != action.event_assertions().len()
    {
        return Err(StateError::Message(
            "validation response result count does not match the certified action".to_owned(),
        ));
    }
    for (result, assertion) in read_results.iter().zip(action.read_assertions()) {
        if result.function != assertion.function.as_str()
            || result.args != assertion.args
            || result.expected != assertion.expected
        {
            return Err(StateError::Message(
                "validation read result does not match the certified action".to_owned(),
            ));
        }
    }
    for (result, assertion) in event_results.iter().zip(action.event_assertions()) {
        if result.event != assertion.event.as_str() || result.min_count != assertion.min_count {
            return Err(StateError::Message(
                "validation event result does not match the certified action".to_owned(),
            ));
        }
    }
    Ok(())
}

fn require_validation_evidence_matches_context(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    read_results: &[ValidationReadResult],
    read_evidence: &[ValidationReadEvidence],
    event_results: &[ValidationEventResult],
    event_evidence: &[ValidationEventEvidence],
) -> StateResult<()> {
    if read_results.len() != read_evidence.len() || event_results.len() != event_evidence.len() {
        return Err(StateError::Message(
            "validation response evidence count does not match assertion results".to_owned(),
        ));
    }
    for (result, evidence) in read_results.iter().zip(read_evidence) {
        require_validation_source_matches_context(context, &evidence.source)?;
        if result != &evidence.result {
            return Err(StateError::Message(
                "validation read evidence does not match its result".to_owned(),
            ));
        }
    }
    for (result, evidence) in event_results.iter().zip(event_evidence) {
        require_validation_source_matches_context(context, &evidence.source)?;
        if result != &evidence.result {
            return Err(StateError::Message(
                "validation event evidence does not match its result".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_code_identity_evidence(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    request: &ValidationCodeIdentityRequest,
    evidence: &ValidationCodeIdentityEvidence,
) -> StateResult<ValidationCodeIdentityReport> {
    if evidence.evidence_version != 1
        || evidence.selector != request.selector
        || !evidence.selector.require_canonical
    {
        return Err(StateError::Message(
            "validation code identity evidence selector does not match the certified request"
                .to_owned(),
        ));
    }
    require_validation_source_matches_context(context, &evidence.source)?;
    let observed_byte_len = u64::try_from(evidence.runtime_bytecode.len()).map_err(|_| {
        StateError::Message("validation runtime bytecode length exceeds u64".to_owned())
    })?;
    let observed_code_hash =
        EvmCodeHash::new(format!("{:?}", keccak256(&evidence.runtime_bytecode)))
            .map_err(|error| StateError::Message(error.to_string()))?;
    if evidence.observed_byte_len != observed_byte_len
        || evidence.observed_code_hash != observed_code_hash
    {
        return Err(StateError::Message(
            "validation code identity evidence does not match recomputed bytecode metadata"
                .to_owned(),
        ));
    }
    Ok(ValidationCodeIdentityReport {
        observed_byte_len,
        observed_code_hash,
    })
}

fn code_identity_matches_expected(
    request: Option<&ValidationCodeIdentityRequest>,
    report: Option<&ValidationCodeIdentityReport>,
) -> bool {
    match (request, report) {
        (None, None) => true,
        (Some(request), Some(report)) => {
            report.observed_byte_len > 0 && report.observed_code_hash == request.expected_code_hash
        }
        _ => false,
    }
}

fn require_validation_source_matches_context(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    source: &ValidationSourceEvidence,
) -> StateResult<()> {
    if source.network_id != context.value().network.network_id.as_str()
        || source.expected_chain_id != context.value().network.expected_chain_id()
        || source.observed_chain_id != context.value().network.expected_chain_id()
        || source.source_ref.trim().is_empty()
        || source.policy_id.trim().is_empty()
    {
        return Err(StateError::Message(
            "validation source evidence does not match the certified EVM context".to_owned(),
        ));
    }
    Ok(())
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
            vec![descriptor_id_for_state::<ContextBoundConfigureContractState>()?],
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::keccak256;
    use mfm_evm_contract_model::{
        ConfiguredContractAnchor, ConfiguredFrom, ContractProfile, ContractProfileId, EvmBlockHash,
        EvmCodeHash, EvmNetworkContext, EvmNetworkId, ExpectedValue, LifecycleKey,
    };
    use mfm_ids::{DigestAlgorithm, DigestBytes};
    use mfm_program::{StateContext, ValidatedConfig};
    use mfm_values::ContextRefValue;
    use serde_json::json;

    struct ValidationFixture {
        context: mfm_program::CertifiedContext<EvmContractContext>,
        input: ContextValidateContractInput,
        state: ContextBoundValidateContractState,
        request: ContextContractValidationReadRequest,
    }

    fn fixture(expected_code_hash: Option<EvmCodeHash>) -> ValidationFixture {
        fixture_with_action(expected_code_hash, ValidateAction::default())
    }

    fn fixture_with_action(
        expected_code_hash: Option<EvmCodeHash>,
        action: ValidateAction,
    ) -> ValidationFixture {
        let value = EvmContractContext {
            lifecycle_key: LifecycleKey::new("validation-state-test").expect("lifecycle key"),
            network: EvmNetworkContext::new(
                EvmNetworkId::new("ethereum-mainnet").expect("network id"),
                1,
            )
            .expect("network context"),
            contract_profile: ContractProfile {
                profile_id: ContractProfileId::new("validation-state-profile").expect("profile id"),
                artifact_digest: None,
                artifact_ref: None,
                interface_digest: None,
                creation_bytecode_digest: None,
                deployed_code_hash: expected_code_hash,
                selector_event_policy_digest: None,
            },
        };
        let mfm_program::StateContextDescriptorSpec::Required(requirement) =
            <EvmContractContext as StateContext>::descriptor().expect("context descriptor")
        else {
            panic!("contract context must be certified");
        };
        let canonical = PlainCanonicalJsonBytes::from_json_str(
            &serde_json::to_string(&value).expect("context JSON"),
        )
        .expect("canonical context");
        let context_ref = mfm_program::CertifiedContextSpec::derive_context_ref(
            &requirement.context_descriptor_id,
            &requirement.schema_id,
            &requirement.semantic_type_id,
            &requirement.canonicalizer_identity,
            &canonical,
        )
        .expect("context ref");
        let context = mfm_program::CertifiedContext::from_certified_spec(
            &mfm_program::CertifiedContextSpec {
                context_ref,
                context_descriptor_id: requirement.context_descriptor_id,
                schema_id: requirement.schema_id,
                semantic_type_id: requirement.semantic_type_id,
                canonicalizer_identity: requirement.canonicalizer_identity,
                canonical_context_digest: canonical.content_digest(),
                canonical_context_byte_len: canonical.as_bytes().len() as u64,
                canonical_context: canonical,
            },
        )
        .expect("certified context");
        let context_ref = ContextRefValue::from(context.context_ref().clone());
        let address = ContractAddress::new("0x1111111111111111111111111111111111111111")
            .expect("contract address");
        let input = ContextValidateContractInput {
            configured: ConfiguredContractInstance {
                lifecycle_version: 1,
                context_ref: context_ref.clone(),
                address: address.clone(),
                configured_from: ConfiguredFrom {
                    deployed_context_ref: context_ref,
                    deployed_address: address,
                },
                anchor: ConfiguredContractAnchor {
                    block_number: 42,
                    block_hash: EvmBlockHash::new(format!("0x{}", "42".repeat(32)))
                        .expect("block hash"),
                },
            },
        };
        let state = ContextBoundValidateContractState::new(
            ValidatedConfig::new(action).expect("validated action"),
        )
        .expect("validation state");
        let request = state.read_request(&input, &context).expect("read request");
        ValidationFixture {
            context,
            input,
            state,
            request,
        }
    }

    fn source() -> ValidationSourceEvidence {
        ValidationSourceEvidence {
            network_id: "ethereum-mainnet".to_owned(),
            expected_chain_id: 1,
            observed_chain_id: 1,
            source_ref: "test-provider".to_owned(),
            policy_id: "test-policy".to_owned(),
        }
    }

    fn code_evidence(
        fixture: &ValidationFixture,
        runtime_bytecode: Vec<u8>,
    ) -> ValidationCodeIdentityEvidence {
        let selector = fixture
            .request
            .code_identity
            .as_ref()
            .expect("code identity request")
            .selector
            .clone();
        ValidationCodeIdentityEvidence {
            evidence_version: 1,
            selector,
            source: source(),
            observed_byte_len: runtime_bytecode.len() as u64,
            observed_code_hash: EvmCodeHash::new(format!("{:?}", keccak256(&runtime_bytecode)))
                .expect("code hash"),
            runtime_bytecode,
        }
    }

    fn response(
        fixture: &ValidationFixture,
        code_identity_evidence: Option<ValidationCodeIdentityEvidence>,
    ) -> ContractValidationReadResponse {
        ContractValidationReadResponse {
            response_version: 1,
            context_ref: ContextRefValue::from(fixture.context.context_ref().clone()),
            configured_input_digest: fixture.request.configured_input_digest.clone(),
            evm_network_context_ref: evm_network_context_ref_for_context(&fixture.context)
                .expect("network context ref"),
            resource_stage: ContractLifecycleStage::Configured,
            observed_chain_id: 1,
            client_version: "test-client".to_owned(),
            code_identity_evidence,
            read_results: Vec::new(),
            event_results: Vec::new(),
            validation_read_evidence: Vec::new(),
            validation_event_evidence: Vec::new(),
        }
    }

    #[test]
    fn code_identity_match_empty_mismatch_and_absence_project_expected_validity() {
        let matching_code = vec![0x60, 0x00];
        let matching_hash =
            EvmCodeHash::new(format!("{:?}", keccak256(&matching_code))).expect("code hash");
        let matching = fixture(Some(matching_hash));
        let report = matching
            .state
            .report_from_response(
                &matching.input,
                response(
                    &matching,
                    Some(code_evidence(&matching, matching_code.clone())),
                ),
                &matching.context,
            )
            .expect("matching report");
        assert!(report.valid);
        assert_eq!(
            report
                .code_identity
                .expect("code identity")
                .observed_byte_len,
            matching_code.len() as u64
        );

        let empty = fixture(Some(
            EvmCodeHash::new(format!("{:?}", keccak256([]))).expect("code hash"),
        ));
        let empty_report = empty
            .state
            .report_from_response(
                &empty.input,
                response(&empty, Some(code_evidence(&empty, Vec::new()))),
                &empty.context,
            )
            .expect("empty report");
        assert!(!empty_report.valid);

        let mismatch = fixture(Some(
            EvmCodeHash::new(format!("{:?}", keccak256([0x60, 0x00]))).expect("code hash"),
        ));
        let mismatch_report = mismatch
            .state
            .report_from_response(
                &mismatch.input,
                response(&mismatch, Some(code_evidence(&mismatch, vec![0x60, 0x01]))),
                &mismatch.context,
            )
            .expect("mismatch report");
        assert!(!mismatch_report.valid);

        let absent = fixture(None);
        assert!(absent.request.code_identity.is_none());
        let absent_report = absent
            .state
            .report_from_response(&absent.input, response(&absent, None), &absent.context)
            .expect("absent report");
        assert!(absent_report.valid);
        assert!(absent_report.code_identity.is_none());
    }

    #[test]
    fn code_identity_rejects_inconsistent_source_selector_and_metadata() {
        let expected =
            EvmCodeHash::new(format!("{:?}", keccak256([0x60, 0x00]))).expect("code hash");
        let fixture = fixture(Some(expected));
        let evidence = code_evidence(&fixture, vec![0x60, 0x00]);

        let mut wrong_source = evidence.clone();
        wrong_source.source.network_id = "wrong-network".to_owned();
        assert!(fixture
            .state
            .report_from_response(
                &fixture.input,
                response(&fixture, Some(wrong_source)),
                &fixture.context,
            )
            .is_err());

        let mut wrong_chain = evidence.clone();
        wrong_chain.source.observed_chain_id = 2;
        assert!(fixture
            .state
            .report_from_response(
                &fixture.input,
                response(&fixture, Some(wrong_chain)),
                &fixture.context,
            )
            .is_err());

        let mut unauthenticated = evidence.clone();
        unauthenticated.source.source_ref.clear();
        assert!(fixture
            .state
            .report_from_response(
                &fixture.input,
                response(&fixture, Some(unauthenticated)),
                &fixture.context,
            )
            .is_err());

        let mut wrong_address = evidence.clone();
        wrong_address.selector.address =
            ContractAddress::new("0x2222222222222222222222222222222222222222")
                .expect("contract address");
        assert!(fixture
            .state
            .report_from_response(
                &fixture.input,
                response(&fixture, Some(wrong_address)),
                &fixture.context,
            )
            .is_err());

        let mut wrong_anchor = evidence.clone();
        wrong_anchor.selector.block_number += 1;
        assert!(fixture
            .state
            .report_from_response(
                &fixture.input,
                response(&fixture, Some(wrong_anchor)),
                &fixture.context,
            )
            .is_err());

        let mut wrong_anchor_hash = evidence.clone();
        wrong_anchor_hash.selector.block_hash =
            EvmBlockHash::new(format!("0x{}", "43".repeat(32))).expect("block hash");
        assert!(fixture
            .state
            .report_from_response(
                &fixture.input,
                response(&fixture, Some(wrong_anchor_hash)),
                &fixture.context,
            )
            .is_err());

        let mut wrong_hash = evidence.clone();
        wrong_hash.observed_code_hash =
            EvmCodeHash::new(format!("0x{}", "00".repeat(32))).expect("code hash");
        assert!(fixture
            .state
            .report_from_response(
                &fixture.input,
                response(&fixture, Some(wrong_hash)),
                &fixture.context,
            )
            .is_err());

        let mut wrong_response_chain = response(&fixture, Some(evidence));
        wrong_response_chain.observed_chain_id = 2;
        assert!(fixture
            .state
            .report_from_response(&fixture.input, wrong_response_chain, &fixture.context)
            .is_err());
    }

    #[test]
    fn code_identity_is_conjunctive_with_retained_read_assertions() {
        let action: ValidateAction = serde_json::from_value(json!({
            "read_assertions": [{
                "function": "owner",
                "args": [],
                "expected": {"json_text": "true"},
            }],
        }))
        .expect("validate action");
        let expected =
            EvmCodeHash::new(format!("{:?}", keccak256([0x60, 0x00]))).expect("code hash");
        let fixture = fixture_with_action(Some(expected), action);
        let expected_value = ExpectedValue::from_json_value(&json!(true)).expect("expected");
        let actual_value = ExpectedValue::from_json_value(&json!(false)).expect("actual");
        let result = ValidationReadResult {
            function: "owner".to_owned(),
            args: Vec::new(),
            expected: expected_value,
            actual: actual_value,
            passed: false,
        };
        let mut response = response(&fixture, Some(code_evidence(&fixture, vec![0x60, 0x00])));
        response.read_results = vec![result.clone()];
        response.validation_read_evidence = vec![ValidationReadEvidence {
            source: source(),
            result,
        }];
        let report = fixture
            .state
            .report_from_response(&fixture.input, response, &fixture.context)
            .expect("report");
        assert!(!report.valid);
    }

    #[test]
    fn validation_without_code_identity_keeps_retained_assertion_failures_false() {
        let action: ValidateAction = serde_json::from_value(json!({
            "read_assertions": [{
                "function": "owner",
                "args": [],
                "expected": {"json_text": "true"},
            }],
        }))
        .expect("validate action");
        let fixture = fixture_with_action(None, action);
        assert!(fixture.request.code_identity.is_none());
        let result = ValidationReadResult {
            function: "owner".to_owned(),
            args: Vec::new(),
            expected: ExpectedValue::from_json_value(&json!(true)).expect("expected"),
            actual: ExpectedValue::from_json_value(&json!(false)).expect("actual"),
            passed: false,
        };
        let mut response = response(&fixture, None);
        response.read_results = vec![result.clone()];
        response.validation_read_evidence = vec![ValidationReadEvidence {
            source: source(),
            result,
        }];
        let report = fixture
            .state
            .report_from_response(&fixture.input, response, &fixture.context)
            .expect("report");
        assert!(!report.valid);
        assert!(report.code_identity.is_none());
    }

    #[test]
    fn configured_anchor_uses_deployment_for_zero_calls_and_last_success_for_many() {
        let context_ref = ContextRefValue::new(mfm_ids::ContextRef::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x42; 32]),
        ));
        let address =
            ContractAddress::new("0x1111111111111111111111111111111111111111").expect("address");
        let input = ContextConfigureContractInput {
            deployed: DeployedContractInstance {
                lifecycle_version: 1,
                context_ref: context_ref.clone(),
                address: address.clone(),
                deployed_block_number: 40,
                deployed_block_hash: EvmBlockHash::new(format!("0x{}", "40".repeat(32)))
                    .expect("block hash"),
            },
        };
        let deploy_anchor = configured_contract_anchor_from_receipts(&input, &[])
            .expect("deployment fallback anchor");
        assert_eq!(deploy_anchor.block_number, 40);

        let receipt = |block_number, block_byte| {
            ContractTransactionReceipt {
            receipt_version: 1,
            context_ref: context_ref.clone(),
            evm_network_context_ref:
                "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_owned(),
            resource_stage: ContractLifecycleStage::Configured,
            transaction_hash: format!("0x{}", "11".repeat(32)),
            block_number,
            block_hash: EvmBlockHash::new(format!("0x{}", format!("{block_byte:02x}").repeat(32)))
                .expect("block hash"),
            status: true,
            receipt_evidence: None,
        }
        };
        let anchor = configured_contract_anchor_from_receipts(
            &input,
            &[receipt(41, 0x41), receipt(42, 0x42)],
        )
        .expect("last receipt anchor");
        assert_eq!(anchor.block_number, 42);
        assert_eq!(anchor.block_hash.as_str(), format!("0x{}", "42".repeat(32)));
    }
}
