use super::*;

pub(super) struct PreparedTransactionInput {
    pub(super) to: Option<Address>,
    pub(super) value_wei: u128,
    pub(super) data: Vec<u8>,
}

pub(super) struct PrepareTransactionsRequest<'a> {
    pub(super) phase: ContractMutationPhase,
    pub(super) context_ref: mfm_values::ContextRefValue,
    pub(super) evm_network_context_ref: String,
    pub(super) resource_stage: ContractLifecycleStage,
    pub(super) network_id: &'a str,
    pub(super) expected_chain_id: u64,
    pub(super) signer_ref: SignerRef,
    pub(super) expected_signer: Address,
    pub(super) expected_signer_text: &'a str,
    pub(super) policy: &'a EvmTransactionPolicy,
    pub(super) poll_interval_ms: u64,
    pub(super) max_receipt_polls: u64,
    pub(super) tx_inputs: Vec<PreparedTransactionInput>,
}

pub(super) trait ContractMutationActionView {
    fn signer(&self) -> &EvmSignerIntent;

    fn transaction(&self) -> &EvmTransactionPolicy;

    fn receipt(&self) -> &ReceiptRetryPolicy;
}

impl ContractMutationActionView for DeployAction {
    fn signer(&self) -> &EvmSignerIntent {
        self.signer()
    }

    fn transaction(&self) -> &EvmTransactionPolicy {
        self.transaction()
    }

    fn receipt(&self) -> &ReceiptRetryPolicy {
        self.receipt()
    }
}

impl ContractMutationActionView for ConfigureAction {
    fn signer(&self) -> &EvmSignerIntent {
        self.signer()
    }

    fn transaction(&self) -> &EvmTransactionPolicy {
        self.transaction()
    }

    fn receipt(&self) -> &ReceiptRetryPolicy {
        self.receipt()
    }
}

#[derive(Clone)]
pub(super) struct ContractMutationNetworkContext<'a> {
    pub(super) context_ref: mfm_values::ContextRefValue,
    pub(super) evm_network_context_ref: String,
    pub(super) resource_stage: ContractLifecycleStage,
    pub(super) network_id: &'a str,
    pub(super) expected_chain_id: u64,
}

impl<'a> ContractMutationNetworkContext<'a> {
    pub(super) fn from_context(
        context: &'a mfm_program::CertifiedContext<EvmContractContext>,
        resource_stage: ContractLifecycleStage,
    ) -> Result<Self> {
        Ok(Self {
            context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
            evm_network_context_ref: evm_network_context_ref(&context.value().network)?,
            resource_stage,
            network_id: context.value().network.network_id.as_str(),
            expected_chain_id: context.value().network.expected_chain_id(),
        })
    }
}

pub(super) struct ResolvedContractMutationAction<'a> {
    network_id: &'a str,
    expected_chain_id: u64,
    context_ref: mfm_values::ContextRefValue,
    evm_network_context_ref: String,
    resource_stage: ContractLifecycleStage,
    signer_ref: SignerRef,
    expected_signer: Address,
    expected_signer_text: &'a str,
}

impl<'a> ResolvedContractMutationAction<'a> {
    fn from_context_and_action(
        network_context: ContractMutationNetworkContext<'a>,
        action: &'a impl ContractMutationActionView,
    ) -> Result<Self> {
        Ok(Self {
            network_id: network_context.network_id,
            expected_chain_id: network_context.expected_chain_id,
            context_ref: network_context.context_ref,
            evm_network_context_ref: network_context.evm_network_context_ref,
            resource_stage: network_context.resource_stage,
            signer_ref: action
                .signer()
                .signer_ref()
                .map_err(EvmContractAdapterError::Model)?,
            expected_signer: action
                .signer()
                .expected_signer_address()
                .map_err(EvmContractAdapterError::Model)?,
            expected_signer_text: action.signer().expected_signer_address_str(),
        })
    }
}

pub(super) fn prepare_transactions_request<'a>(
    phase: ContractMutationPhase,
    network_context: ContractMutationNetworkContext<'a>,
    action: &'a impl ContractMutationActionView,
    tx_inputs: Vec<PreparedTransactionInput>,
) -> Result<PrepareTransactionsRequest<'a>> {
    let resolved =
        ResolvedContractMutationAction::from_context_and_action(network_context, action)?;
    Ok(PrepareTransactionsRequest {
        phase,
        context_ref: resolved.context_ref,
        evm_network_context_ref: resolved.evm_network_context_ref,
        resource_stage: resolved.resource_stage,
        network_id: resolved.network_id,
        expected_chain_id: resolved.expected_chain_id,
        signer_ref: resolved.signer_ref,
        expected_signer: resolved.expected_signer,
        expected_signer_text: resolved.expected_signer_text,
        policy: action.transaction(),
        poll_interval_ms: action.receipt().poll_interval_ms(),
        max_receipt_polls: action.receipt().max_receipt_polls(),
        tx_inputs,
    })
}

pub(super) fn prepared_mutation_reconstruction<'a>(
    evidence: &'a PreparedContractInvocation,
    phase: ContractMutationPhase,
    network_context: ContractMutationNetworkContext<'a>,
    action: &'a impl ContractMutationActionView,
    tx_inputs: Vec<PreparedTransactionInput>,
) -> Result<PreparedMutationReconstruction<'a>> {
    let resolved =
        ResolvedContractMutationAction::from_context_and_action(network_context, action)?;
    Ok(PreparedMutationReconstruction {
        evidence,
        phase,
        network_id: resolved.network_id,
        expected_chain_id: resolved.expected_chain_id,
        context_ref: resolved.context_ref,
        evm_network_context_ref: resolved.evm_network_context_ref,
        resource_stage: resolved.resource_stage,
        signer_ref: resolved.signer_ref,
        expected_signer: resolved.expected_signer,
        expected_signer_text: resolved.expected_signer_text,
        tx_inputs,
    })
}

pub(super) struct PreparedTransaction {
    pub(super) style: PreparedContractTransactionStyle,
    pub(super) request: EvmSigningRequest,
    pub(super) max_fee_per_gas: Option<u128>,
    pub(super) max_priority_fee_per_gas: Option<u128>,
    pub(super) gas_price: Option<u128>,
}

pub(super) fn lifecycle_evidence_ref(
    evidence: &CapabilityArtifactEvidenceRef,
) -> LifecycleArtifactEvidenceRef {
    let store_evidence = evidence.clone().into_store();
    // Store evidence already carries typed identity fields; hashing is total for this shape.
    let evidence_hash = store_evidence
        .evidence_hash()
        .expect("capability artifact evidence hash");
    LifecycleArtifactEvidenceRef::new(
        evidence.artifact_id.clone(),
        evidence.digest.clone(),
        evidence_hash,
        evidence.byte_len,
        evidence.schema_id.clone(),
        evidence.semantic_type_id.clone(),
    )
}

pub(super) fn deploy_contract_address_from_prepared(
    prepared: &PreparedContractInvocation,
) -> Result<String> {
    ensure_prepared_invocation_public(prepared)?;
    if prepared.phase != ContractMutationPhase::Deploy || prepared.transactions.len() != 1 {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }
    let transaction = &prepared.transactions[0];
    if transaction.to_address.is_some() {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }
    let signer = parse_address(&prepared.expected_signer_address, "expected_signer_address")
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    normalize_address(&format!(
        "{:?}",
        created_contract_address(signer, transaction.nonce)
    ))
    .map_err(|error| EvmContractAdapterError::Model(error.message))
}

pub(super) fn created_contract_address(sender: Address, nonce: u64) -> Address {
    let encoded = rlp_encode_list(&[sender.as_slice().to_vec(), u64_to_min_be(nonce)]);
    let hash = keccak256(encoded);
    Address::from_slice(&hash.as_slice()[12..])
}

/// Validates that prepared invocation evidence has no live or secret-bearing surface.
pub fn ensure_prepared_invocation_public(prepared: &PreparedContractInvocation) -> Result<()> {
    if prepared.prepared_version != 1 {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }
    EvmNetworkId::new(&prepared.network_id)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    SignerRef::new(&prepared.signer_ref)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    ensure_canonical_address(&prepared.expected_signer_address)?;
    ReceiptRetryPolicy::new(prepared.poll_interval_ms, prepared.max_receipt_polls)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    ContentDigest::parse(&prepared.evm_network_context_ref)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;

    for (index, transaction) in prepared.transactions.iter().enumerate() {
        ensure_prepared_transaction_public(prepared.expected_chain_id, index, transaction)?;
    }
    Ok(())
}

pub(super) fn ensure_prepared_transaction_public(
    expected_chain_id: u64,
    index: usize,
    transaction: &PreparedContractTransactionEvidence,
) -> Result<()> {
    if transaction.index != index as u64 || transaction.chain_id != expected_chain_id {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }
    if let Some(to_address) = transaction.to_address.as_deref() {
        ensure_canonical_address(to_address)?;
    }
    ensure_canonical_quantity(&transaction.value_wei, "value_wei")?;
    ensure_canonical_quantity_optional(transaction.max_fee_per_gas.as_deref(), "max_fee_per_gas")?;
    ensure_canonical_quantity_optional(
        transaction.max_priority_fee_per_gas.as_deref(),
        "max_priority_fee_per_gas",
    )?;
    ensure_canonical_quantity_optional(transaction.gas_price.as_deref(), "gas_price")?;
    match transaction.style {
        PreparedContractTransactionStyle::Eip1559 => {
            if transaction.max_fee_per_gas.is_none()
                || transaction.max_priority_fee_per_gas.is_none()
                || transaction.gas_price.is_some()
            {
                return Err(EvmContractAdapterError::InvalidPreparedInvocation);
            }
        }
        PreparedContractTransactionStyle::Legacy => {
            if transaction.gas_price.is_none()
                || transaction.max_fee_per_gas.is_some()
                || transaction.max_priority_fee_per_gas.is_some()
            {
                return Err(EvmContractAdapterError::InvalidPreparedInvocation);
            }
        }
    }
    ContentDigest::parse(&transaction.data_digest)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    parse_b256_hex(&transaction.signing_digest)?;
    parse_b256_hex(&transaction.expected_transaction_hash)?;
    Ok(())
}

pub(super) fn ensure_canonical_address(value: &str) -> Result<()> {
    let normalized =
        normalize_address(value).map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    if normalized == value {
        Ok(())
    } else {
        Err(EvmContractAdapterError::InvalidPreparedInvocation)
    }
}

pub(super) fn ensure_canonical_quantity(value: &str, field: &'static str) -> Result<()> {
    let parsed = parse_u128_quantity(value, field)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    if parsed.to_string() == value {
        Ok(())
    } else {
        Err(EvmContractAdapterError::InvalidPreparedInvocation)
    }
}

pub(super) fn ensure_canonical_quantity_optional(
    value: Option<&str>,
    field: &'static str,
) -> Result<()> {
    if let Some(value) = value {
        ensure_canonical_quantity(value, field)?;
    }
    Ok(())
}

pub(super) fn parse_b256_hex(value: &str) -> Result<B256> {
    value
        .parse::<B256>()
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)
}

/// Identifies side-effect intents owned by this adapter's replay verifier.
pub fn is_contract_state_replay_intent(
    intent: &side_effect::IntentPersisted,
) -> replay::Result<bool> {
    let adapter_kind =
        mfm_state_evm_contracts::contract_states_adapter_kind().map_err(replay_adapter_error)?;
    let adapter_version =
        mfm_state_evm_contracts::contract_states_adapter_version().map_err(replay_adapter_error)?;
    Ok(intent.adapter_kind == adapter_kind && intent.adapter_version == adapter_version)
}

pub(super) fn contract_state_side_effect_missing(phase: &str) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMissing,
        format!("missing contract state {phase} evidence"),
    )
}

pub(super) fn deploy_action_data(
    action: &DeployAction,
    artifact: &ContractArtifactConfig,
) -> Result<Vec<u8>> {
    let (abi, bytecode) = parse_artifact(artifact).map_err(EvmContractAdapterError::Model)?;
    constructor_data(&abi, &bytecode, action.constructor_args())
        .map_err(EvmContractAdapterError::Model)
}

pub(super) fn configure_action_requires_artifact(action: &ConfigureAction) -> bool {
    !action.calls().is_empty()
}

pub(super) fn configure_action_transaction_inputs(
    action: &ConfigureAction,
    artifact: Option<&ContractArtifactConfig>,
    contract_address: &str,
) -> Result<Vec<PreparedTransactionInput>> {
    if !configure_action_requires_artifact(action) {
        return Ok(Vec::new());
    }
    let artifact = artifact.ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let (abi, _) = parse_artifact(artifact).map_err(EvmContractAdapterError::Model)?;
    let to = parse_address(contract_address, "contract_address")
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    action
        .calls()
        .iter()
        .map(|call| configure_transaction_input(&abi, to, call))
        .collect()
}

pub(super) fn configure_transaction_input(
    abi: &mfm_evm_contract_model::ParsedAbi,
    to: Address,
    call: &ContractCallConfig,
) -> Result<PreparedTransactionInput> {
    let (data, _) = resolve_function_call(abi, call.function.as_str(), &call.args)
        .map_err(EvmContractAdapterError::Model)?;
    Ok(PreparedTransactionInput {
        to: Some(to),
        value_wei: parse_optional_wei(call.value_wei.as_ref().map(|value| value.as_str()))?,
        data,
    })
}

pub(super) fn reconstruct_prepared_mutation(
    request: PreparedMutationReconstruction<'_>,
) -> Result<PreparedContractMutation> {
    let PreparedMutationReconstruction {
        evidence,
        phase,
        network_id,
        expected_chain_id,
        context_ref,
        evm_network_context_ref,
        resource_stage,
        signer_ref,
        expected_signer,
        expected_signer_text,
        tx_inputs,
    } = request;
    ensure_prepared_invocation_public(evidence)?;
    let expected_signer_address = normalize_address(expected_signer_text)
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    if evidence.phase != phase
        || evidence.network_id != network_id
        || evidence.expected_chain_id != expected_chain_id
        || evidence.context_ref != context_ref
        || evidence.evm_network_context_ref != evm_network_context_ref
        || evidence.resource_stage != resource_stage
        || evidence.signer_ref != signer_ref.to_string()
        || evidence.expected_signer_address != expected_signer_address
        || evidence.transactions.len() != tx_inputs.len()
    {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }

    let mut signing_requests = Vec::with_capacity(tx_inputs.len());
    for (index, (input, transaction)) in tx_inputs
        .into_iter()
        .zip(evidence.transactions.iter())
        .enumerate()
    {
        let expected_to = input.to.map(|address| format!("{address:?}"));
        if transaction.index != index as u64
            || transaction.chain_id != expected_chain_id
            || transaction.to_address != expected_to
            || transaction.value_wei != input.value_wei.to_string()
            || transaction.data_digest != digest_bytes(&input.data).to_string()
            || transaction.data_len != input.data.len() as u64
        {
            return Err(EvmContractAdapterError::InvalidPreparedInvocation);
        }

        let signing_request = match transaction.style {
            PreparedContractTransactionStyle::Eip1559 => {
                let max_fee_per_gas = required_prepared_quantity(
                    transaction.max_fee_per_gas.as_deref(),
                    "max_fee_per_gas",
                )?;
                let max_priority_fee_per_gas = required_prepared_quantity(
                    transaction.max_priority_fee_per_gas.as_deref(),
                    "max_priority_fee_per_gas",
                )?;
                if transaction.gas_price.is_some() {
                    return Err(EvmContractAdapterError::InvalidPreparedInvocation);
                }
                EvmSigningRequest::eip1559(
                    signer_ref.clone(),
                    Eip1559TxToSign {
                        to: input.to,
                        value_wei: input.value_wei,
                        chain_id: expected_chain_id,
                        nonce: transaction.nonce,
                        max_fee_per_gas,
                        max_priority_fee_per_gas,
                        gas_limit: transaction.gas_limit,
                        data: input.data,
                    },
                    expected_signer,
                )?
            }
            PreparedContractTransactionStyle::Legacy => {
                let gas_price_wei =
                    required_prepared_quantity(transaction.gas_price.as_deref(), "gas_price")?;
                if transaction.max_fee_per_gas.is_some()
                    || transaction.max_priority_fee_per_gas.is_some()
                {
                    return Err(EvmContractAdapterError::InvalidPreparedInvocation);
                }
                EvmSigningRequest::legacy(
                    signer_ref.clone(),
                    LegacyTxToSign {
                        to: input.to,
                        value_wei: input.value_wei,
                        chain_id: expected_chain_id,
                        nonce: transaction.nonce,
                        gas_price_wei,
                        gas_limit: transaction.gas_limit,
                        data: input.data,
                    },
                    expected_signer,
                )?
            }
        };
        if transaction.signing_digest != format!("{:?}", signing_request.signing_hash()) {
            return Err(EvmContractAdapterError::InvalidPreparedInvocation);
        }
        parse_prepared_transaction_hash(&transaction.expected_transaction_hash)?;
        signing_requests.push(signing_request);
    }

    PreparedContractMutation::new(evidence.clone(), signing_requests)
}

pub(super) struct PreparedMutationReconstruction<'a> {
    evidence: &'a PreparedContractInvocation,
    phase: ContractMutationPhase,
    network_id: &'a str,
    expected_chain_id: u64,
    context_ref: mfm_values::ContextRefValue,
    evm_network_context_ref: String,
    resource_stage: ContractLifecycleStage,
    signer_ref: SignerRef,
    expected_signer: Address,
    expected_signer_text: &'a str,
    tx_inputs: Vec<PreparedTransactionInput>,
}

pub(super) fn required_prepared_quantity(value: Option<&str>, field: &'static str) -> Result<u128> {
    optional_policy_quantity(value, field)?
        .ok_or(EvmContractAdapterError::InvalidPreparedInvocation)
}

pub(super) fn parse_prepared_transaction_hash(value: &str) -> Result<B256> {
    parse_b256_hex(value)
}

pub(super) fn verify_receipt_response(
    expected_hash: B256,
    response: EvmReceiptReadResponse,
) -> Result<EvmReceiptReadResponse> {
    if response.transaction_hash != expected_hash {
        return Err(EvmContractAdapterError::TransactionHashMismatch);
    }
    if !response.status {
        return Err(EvmContractAdapterError::TransactionFailed);
    }
    Ok(response)
}

pub(super) fn prepared_anchor_submissions(
    prepared: &PreparedContractInvocation,
) -> Result<ContractTransactionSubmissions> {
    let transaction_hashes = prepared_transaction_hashes(prepared)?;
    Ok(transaction_hashes_to_submissions(
        prepared,
        transaction_hashes,
    ))
}

pub(super) fn prepared_transaction_hashes(
    prepared: &PreparedContractInvocation,
) -> Result<Vec<B256>> {
    ensure_prepared_invocation_public(prepared)?;
    prepared
        .transactions
        .iter()
        .map(|transaction| parse_prepared_transaction_hash(&transaction.expected_transaction_hash))
        .collect::<Result<Vec<_>>>()
}

pub(super) fn verify_prepared_submissions(
    prepared: &PreparedContractInvocation,
    submissions: &ContractTransactionSubmissions,
) -> Result<Vec<B256>> {
    if submissions.submissions_version != 1
        || submissions.context_ref != prepared.context_ref
        || submissions.evm_network_context_ref != prepared.evm_network_context_ref
        || submissions.resource_stage != prepared.resource_stage
    {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }
    verify_prepared_submission_transactions(prepared, &submissions.transactions)
}

pub(super) fn verify_prepared_submission_transactions(
    prepared: &PreparedContractInvocation,
    submissions: &[ContractTransactionSubmission],
) -> Result<Vec<B256>> {
    ensure_prepared_invocation_public(prepared)?;
    if prepared.transactions.len() != submissions.len() {
        return Err(EvmContractAdapterError::TransactionHashMismatch);
    }
    let mut transaction_hashes = Vec::with_capacity(submissions.len());
    for (prepared_transaction, submission) in prepared.transactions.iter().zip(submissions) {
        if submission.submission_version != 1
            || submission.context_ref != prepared.context_ref
            || submission.evm_network_context_ref != prepared.evm_network_context_ref
            || submission.resource_stage != prepared.resource_stage
        {
            return Err(EvmContractAdapterError::InvalidPreparedInvocation);
        }
        let expected_hash =
            parse_prepared_transaction_hash(&prepared_transaction.expected_transaction_hash)?;
        let submitted_hash = parse_b256_hex(&submission.transaction_hash)
            .map_err(|_| EvmContractAdapterError::TransactionHashMismatch)?;
        if submitted_hash != expected_hash {
            return Err(EvmContractAdapterError::TransactionHashMismatch);
        }
        transaction_hashes.push(expected_hash);
    }
    Ok(transaction_hashes)
}

pub(super) fn transaction_hashes_to_submissions(
    prepared: &PreparedContractInvocation,
    transaction_hashes: Vec<B256>,
) -> ContractTransactionSubmissions {
    ContractTransactionSubmissions {
        submissions_version: 1,
        context_ref: prepared.context_ref.clone(),
        evm_network_context_ref: prepared.evm_network_context_ref.clone(),
        resource_stage: prepared.resource_stage,
        transactions: transaction_hashes
            .into_iter()
            .map(|transaction_hash| ContractTransactionSubmission {
                submission_version: 1,
                context_ref: prepared.context_ref.clone(),
                evm_network_context_ref: prepared.evm_network_context_ref.clone(),
                resource_stage: prepared.resource_stage,
                transaction_hash: format!("{transaction_hash:?}"),
                signer_public_key: None,
            })
            .collect(),
    }
}

pub(super) enum PreparedSubmissionReconciliation {
    Observed(ContractTransactionSubmissions),
    NotObserved(ContractTransactionSubmissions),
    NotSubmitted(ContractNotSubmittedProof),
    Indeterminate(ContractTransactionSubmissions),
}

pub(super) type ContractSubmissionDecision = SideEffectSubmissionDecision<
    ContractTransactionSubmissions,
    ContractTransactionSubmissions,
    ContractNotSubmittedProof,
    ContractTransactionSubmissions,
>;

pub(super) fn transaction_hash_mismatch_ambiguity(
    evidence: ContractTransactionSubmissions,
) -> Result<ContractSubmissionDecision> {
    Ok(SideEffectSubmissionDecision::Ambiguous {
        ambiguity_code: events::AmbiguityCode::new("mfm.evm.transaction_hash_mismatch")
            .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?,
        evidence,
    })
}

pub(super) async fn submit_or_recover_contract_submission(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractMutation,
    action: SideEffectProtocolAction,
) -> Result<ContractSubmissionDecision> {
    if matches!(
        action,
        SideEffectProtocolAction::SubmitOrRecoverSubmission { .. }
    ) {
        match runtime
            .adapter()
            .reconcile_prepared_submission(prepared.evidence())
            .await?
        {
            PreparedSubmissionReconciliation::Observed(submissions) => {
                return Ok(SideEffectSubmissionDecision::Observed(submissions));
            }
            PreparedSubmissionReconciliation::Indeterminate(anchor_submissions) => {
                return Ok(SideEffectSubmissionDecision::Unknown(anchor_submissions));
            }
            PreparedSubmissionReconciliation::NotSubmitted(proof) => {
                return Ok(SideEffectSubmissionDecision::NotSubmitted(proof));
            }
            PreparedSubmissionReconciliation::NotObserved(anchor_submissions) => {
                return submit_prepared_contract_submission(runtime, prepared, anchor_submissions)
                    .await;
            }
        }
    }

    submit_prepared_contract_submission(
        runtime,
        prepared,
        prepared_anchor_submissions(prepared.evidence())?,
    )
    .await
}

pub(super) async fn submit_prepared_contract_submission(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractMutation,
    ambiguity_evidence: ContractTransactionSubmissions,
) -> Result<ContractSubmissionDecision> {
    match runtime.adapter().submit_prepared(prepared).await {
        Ok(transactions) => Ok(SideEffectSubmissionDecision::Observed(
            ContractTransactionSubmissions {
                submissions_version: 1,
                context_ref: prepared.evidence().context_ref.clone(),
                evm_network_context_ref: prepared.evidence().evm_network_context_ref.clone(),
                resource_stage: prepared.evidence().resource_stage,
                transactions,
            },
        )),
        Err(EvmContractAdapterError::TransactionHashMismatch) => {
            transaction_hash_mismatch_ambiguity(ambiguity_evidence)
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn recover_unknown_prepared_contract_submission(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractMutation,
) -> mfm_runtime::Result<
    SideEffectUnknownSubmissionDecision<
        ContractTransactionSubmissions,
        ContractNotSubmittedProof,
        ContractTransactionSubmissions,
    >,
> {
    match runtime
        .adapter()
        .reconcile_prepared_submission(prepared.evidence())
        .await?
    {
        PreparedSubmissionReconciliation::Observed(submissions) => {
            Ok(SideEffectUnknownSubmissionDecision::Observed(submissions))
        }
        PreparedSubmissionReconciliation::NotSubmitted(proof) => {
            Ok(SideEffectUnknownSubmissionDecision::NotSubmitted(proof))
        }
        PreparedSubmissionReconciliation::Indeterminate(_)
        | PreparedSubmissionReconciliation::NotObserved(_) => {
            Ok(SideEffectUnknownSubmissionDecision::StillUnknown)
        }
    }
}

pub(super) fn parse_optional_wei(value: Option<&str>) -> Result<u128> {
    value
        .map(|raw| parse_u128_quantity(raw, "value_wei").map_err(|error| error.message))
        .transpose()
        .map_err(EvmContractAdapterError::Model)
        .map(|value| value.unwrap_or(0))
}

pub(super) fn optional_policy_quantity(
    value: Option<&str>,
    field: &'static str,
) -> Result<Option<u128>> {
    value
        .map(|raw| parse_u128_quantity(raw, field).map_err(|error| error.message))
        .transpose()
        .map_err(EvmContractAdapterError::Model)
}

pub(super) fn block_selector(
    selector: Option<&mfm_evm_contract_model::BlockSelector>,
    default_latest: bool,
) -> Result<EvmBlockSelector> {
    use mfm_evm_contract_model::{BlockSelector, BlockTag};

    match selector {
        None if default_latest => Ok(EvmBlockSelector::Latest),
        None => Ok(EvmBlockSelector::Number(0)),
        Some(BlockSelector::Number { number }) => Ok(EvmBlockSelector::Number(*number)),
        Some(BlockSelector::Tag {
            tag: BlockTag::Earliest,
        }) => Ok(EvmBlockSelector::Number(0)),
        Some(BlockSelector::Tag {
            tag: BlockTag::Latest,
        }) => Ok(EvmBlockSelector::Latest),
        Some(BlockSelector::Tag {
            tag: BlockTag::Pending,
        }) => Ok(EvmBlockSelector::Pending),
        Some(BlockSelector::Tag {
            tag: BlockTag::Safe | BlockTag::Finalized,
        }) => Err(EvmContractAdapterError::UnsupportedBlockTag),
    }
}

pub(super) fn digest_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

pub(super) fn evm_network_context_ref(network: &EvmNetworkContext) -> Result<String> {
    let json = serde_json::to_string(network)
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.content_digest().to_string())
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))
}

pub(super) fn ensure_deployed_input_context(
    input: &ContextConfigureContractInput,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> Result<()> {
    if input.deployed.context_ref.as_context_ref() == context.context_ref() {
        Ok(())
    } else {
        Err(EvmContractAdapterError::ContextMismatch)
    }
}

pub(super) fn ensure_configured_input_context(
    input: &ContextValidateContractInput,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> Result<()> {
    if input.configured.context_ref.as_context_ref() == context.context_ref()
        && input
            .configured
            .configured_from
            .deployed_context_ref
            .as_context_ref()
            == context.context_ref()
    {
        Ok(())
    } else {
        Err(EvmContractAdapterError::ContextMismatch)
    }
}

pub(super) fn public_key_hex(public_key: Option<&PublicKeyBytes>) -> Option<String> {
    public_key.map(|key| bytes_to_hex_prefixed(key.as_bytes()))
}

pub(super) fn ensure_schema(
    actual: &SchemaId,
    expected: &SchemaId,
    label: &'static str,
) -> replay::Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            format!("{label} schema did not match contract-state schema"),
        ))
    }
}

pub(super) fn replay_adapter_error(error: impl fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

pub(super) fn replay_model_error(error: impl fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

pub(super) fn replay_contract_mismatch(message: &'static str) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::SideEffectMismatch, message)
}

pub(super) fn replay_value_error(error: mfm_values::ValueError) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

pub(super) fn replay_json_error(error: serde_json::Error) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

pub(super) fn ensure_replay_confirmation_depth(
    confirmations: u64,
    required_depth: u64,
) -> replay::Result<()> {
    if confirmations >= required_depth {
        Ok(())
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            format!(
                "contract-state confirmation depth {confirmations} is below certified depth {required_depth}"
            ),
        ))
    }
}
