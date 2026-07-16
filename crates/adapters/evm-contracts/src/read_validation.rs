use super::*;

pub(super) async fn validate_context_contract_with_reads(
    reads: EvmContractReadProviders<'_>,
    action: &ValidatedConfig<ValidateAction>,
    input: &ContextValidateContractInput,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    artifact: Option<&ContractArtifactConfig>,
    request: &ContextContractValidationReadRequest,
) -> Result<ContractValidationReadResponse> {
    let expected =
        ContextBoundValidateContractState::new(action.clone())?.read_request(input, context)?;
    if &expected != request {
        return Err(EvmContractAdapterError::IntentMismatch);
    }
    ensure_configured_input_context(input, context)?;

    let chain = read_chain_identity(reads.chain_identity).await?;
    ensure_evm_source_matches_binding(
        &chain.evidence,
        context.value().network.network_id.as_str(),
        context.value().network.expected_chain_id(),
    )?;
    if chain.chain_id != context.value().network.expected_chain_id() {
        return Err(EvmContractAdapterError::ContextMismatch);
    }
    let code_identity_evidence = match &request.code_identity {
        Some(request) => Some(
            read_code_identity(
                reads.code,
                request,
                context.value().network.network_id.as_str(),
                context.value().network.expected_chain_id(),
            )
            .await?,
        ),
        None => None,
    };
    let assertion_context = prepare_validation_assertion_context(
        artifact,
        input.configured.address.as_str(),
        validation_assertions_required(&request.read_assertions, &request.event_assertions),
    )?;
    let evaluated = evaluate_assertions(
        reads,
        assertion_context.as_ref(),
        &request.read_assertions,
        &request.event_assertions,
    )
    .await?;

    Ok(ContractValidationReadResponse {
        response_version: 1,
        context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
        configured_input_digest: request.configured_input_digest.clone(),
        evm_network_context_ref: evm_network_context_ref(&context.value().network)?,
        resource_stage: ContractLifecycleStage::Configured,
        observed_chain_id: chain.chain_id,
        client_version: chain.client_version.unwrap_or_else(|| "unknown".to_owned()),
        code_identity_evidence,
        read_results: evaluated.read_results,
        event_results: evaluated.event_results,
        validation_read_evidence: evaluated.read_evidence,
        validation_event_evidence: evaluated.event_evidence,
    })
}

async fn read_code_identity(
    provider: &dyn EvmCodeReadProvider,
    request: &mfm_evm_contract_model::ValidationCodeIdentityRequest,
    network_id: &str,
    expected_chain_id: u64,
) -> Result<ValidationCodeIdentityEvidence> {
    if !request.selector.require_canonical {
        return Err(EvmContractAdapterError::InvalidCodeIdentityEvidence);
    }
    let address = parse_address(request.selector.address.as_str(), "contract_address")
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    let block_hash = request
        .selector
        .block_hash
        .as_str()
        .parse::<B256>()
        .map_err(|_| EvmContractAdapterError::InvalidCodeIdentityEvidence)?;
    let response = provider
        .read_code(&EvmCodeReadRequest::new(
            address,
            EvmBlockSelector::Hash(block_hash),
        ))
        .await?;
    ensure_evm_source_matches_binding(&response.evidence, network_id, expected_chain_id)?;
    let observed_byte_len = u64::try_from(response.code.len())
        .map_err(|_| EvmContractAdapterError::InvalidCodeIdentityEvidence)?;
    let recomputed_hash = keccak256(&response.code);
    if response.code_hash != recomputed_hash {
        return Err(EvmContractAdapterError::InvalidCodeIdentityEvidence);
    }
    let observed_code_hash = EvmCodeHash::new(format!("{recomputed_hash:?}"))
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
    Ok(ValidationCodeIdentityEvidence {
        evidence_version: 1,
        selector: request.selector.clone(),
        source: validation_source_evidence(&response.evidence),
        runtime_bytecode: response.code,
        observed_byte_len,
        observed_code_hash,
    })
}

pub(super) fn context_stage_for_lifecycle_stage(
    stage: ContractLifecycleStage,
) -> &'static mfm_ids::ContextStage {
    match stage {
        ContractLifecycleStage::Deployed => deployed_contract_stage(),
        ContractLifecycleStage::Configured => configured_contract_stage(),
    }
}

async fn read_chain_identity(
    provider: &dyn EvmChainIdentityProvider,
) -> Result<EvmChainIdentityResponse> {
    provider
        .chain_identity(&EvmChainIdentityRequest::new())
        .await
        .map_err(Into::into)
}

struct ValidationAssertionContext {
    abi: ParsedAbi,
    address: Address,
}

struct EvaluatedAssertions {
    read_results: Vec<ValidationReadResult>,
    event_results: Vec<ValidationEventResult>,
    read_evidence: Vec<ValidationReadEvidence>,
    event_evidence: Vec<ValidationEventEvidence>,
}

fn prepare_validation_assertion_context(
    artifact: Option<&ContractArtifactConfig>,
    contract_address: &str,
    required: bool,
) -> Result<Option<ValidationAssertionContext>> {
    if !required {
        return Ok(None);
    }
    let artifact = artifact.ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let (abi, _) = parse_artifact(artifact).map_err(EvmContractAdapterError::Model)?;
    let address = parse_address(contract_address, "contract_address")
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    Ok(Some(ValidationAssertionContext { abi, address }))
}

pub(super) fn validation_assertions_required(
    read_assertions: &[ReadAssertionConfig],
    event_assertions: &[EventAssertionConfig],
) -> bool {
    !read_assertions.is_empty() || !event_assertions.is_empty()
}

async fn evaluate_assertions(
    reads: EvmContractReadProviders<'_>,
    context: Option<&ValidationAssertionContext>,
    read_assertions: &[ReadAssertionConfig],
    event_assertions: &[EventAssertionConfig],
) -> Result<EvaluatedAssertions> {
    if !validation_assertions_required(read_assertions, event_assertions) {
        return Ok(EvaluatedAssertions {
            read_results: Vec::new(),
            event_results: Vec::new(),
            read_evidence: Vec::new(),
            event_evidence: Vec::new(),
        });
    }
    let context = context.ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let (prepared_reads, prepared_events) =
        prepare_validate_assertions(&context.abi, read_assertions, event_assertions)
            .map_err(EvmContractAdapterError::Model)?;

    let mut read_results = Vec::with_capacity(prepared_reads.len());
    let mut read_evidence = Vec::with_capacity(prepared_reads.len());
    for (assertion, prepared) in read_assertions.iter().zip(prepared_reads.iter()) {
        let response = reads
            .call
            .read_call(&EvmCallReadRequest::new(
                context.address,
                hex_to_bytes(&prepared.data_hex)
                    .map_err(|error| EvmContractAdapterError::Model(error.message))?,
                EvmBlockSelector::Latest,
            ))
            .await?;
        let actual_json = decode_single_output_to_json(
            &prepared.outputs,
            &bytes_to_hex_prefixed(&response.return_data),
        )
        .map_err(EvmContractAdapterError::Model)?;
        let actual =
            ExpectedValue::from_json_value(&actual_json).map_err(EvmContractAdapterError::Model)?;
        let result = ValidationReadResult {
            function: assertion.function.to_string(),
            args: assertion.args.clone(),
            expected: assertion.expected.clone(),
            passed: expected_matches(&actual, &assertion.expected),
            actual,
        };
        read_evidence.push(ValidationReadEvidence {
            source: validation_source_evidence(&response.evidence),
            result: result.clone(),
        });
        read_results.push(result);
    }

    let mut event_results = Vec::with_capacity(prepared_events.len());
    let mut event_evidence = Vec::with_capacity(prepared_events.len());
    for (assertion, prepared) in event_assertions.iter().zip(prepared_events.iter()) {
        let topic = prepared
            .topic0_hex
            .parse::<B256>()
            .map_err(|_| EvmContractAdapterError::Model("invalid event topic".to_owned()))?;
        let logs = reads
            .logs
            .read_logs(&EvmLogsReadRequest::new(
                block_selector(assertion.from_block.as_ref(), false)?,
                block_selector(assertion.to_block.as_ref(), true)?,
                Some(context.address),
                vec![topic],
            ))
            .await?;
        let observed_count = logs.logs.len() as u64;
        let result = ValidationEventResult {
            event: prepared.event.clone(),
            min_count: prepared.min_count,
            observed_count,
            passed: observed_count >= prepared.min_count,
        };
        event_evidence.push(ValidationEventEvidence {
            source: validation_source_evidence(&logs.evidence),
            result: result.clone(),
        });
        event_results.push(result);
    }

    Ok(EvaluatedAssertions {
        read_results,
        event_results,
        read_evidence,
        event_evidence,
    })
}

fn validation_source_evidence(evidence: &RedactedEvmSourceEvidence) -> ValidationSourceEvidence {
    ValidationSourceEvidence {
        network_id: evidence.network_id.to_string(),
        expected_chain_id: evidence.expected_chain_id,
        observed_chain_id: evidence.observed_chain_id,
        source_ref: evidence.source_ref.to_string(),
        policy_id: evidence.policy_id.to_string(),
    }
}
