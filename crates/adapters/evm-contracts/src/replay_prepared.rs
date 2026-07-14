use super::*;

pub(crate) fn replay_prepared_invocation(
    prepared: Option<&replay::PreparedInvocationReplayEvidence>,
) -> replay::Result<PreparedContractInvocation> {
    let prepared = prepared.ok_or_else(|| contract_lifecycle_side_effect_missing("prepared"))?;
    let evidence: PreparedContractInvocation =
        serde_json::from_slice(&prepared.artifact_bytes).map_err(replay_json_error)?;
    ensure_prepared_invocation_public(&evidence).map_err(replay_adapter_error)?;
    Ok(evidence)
}

pub(crate) fn verify_prepared_matches_certified_side_effect_context(
    certified: &replay::CertifiedSideEffectContext,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    ensure_prepared_invocation_public(prepared).map_err(replay_adapter_error)?;
    let expected_stage = context_stage_for_lifecycle_stage(prepared.resource_stage);
    let phase_stage_matches = matches!(
        (prepared.phase, prepared.resource_stage),
        (
            ContractMutationPhase::Deploy,
            ContractLifecycleStage::Deployed
        ) | (
            ContractMutationPhase::Configure,
            ContractLifecycleStage::Configured
        )
    );
    let (
        spec::NodeContextSpec::Required { context_ref },
        spec::CellContextSpec::Bound {
            context_ref: output_context_ref,
            resource_kind,
            stage,
            ..
        },
    ) = (&certified.node_context, &certified.output_context)
    else {
        return Err(replay_contract_mismatch(
            "contract lifecycle side effect is missing certified context authority",
        ));
    };
    if context_ref != output_context_ref
        || prepared.context_ref.as_context_ref() != context_ref
        || resource_kind != mfm_evm_contract_model::contract_instance_resource_kind()
        || stage != expected_stage
        || !phase_stage_matches
    {
        return Err(replay_contract_mismatch(
            "prepared invocation context does not match certified node context",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) enum VerifiedContractSideEffectIntent {
    Deploy(Box<ContextContractDeployIntent>),
    Configure(Box<ContextContractConfigureIntent>),
}

pub(crate) fn verify_replay_intent_matches_prepared(
    intent: &replay::SideEffectIntentReplayEvidence,
    prepared: &PreparedContractInvocation,
) -> replay::Result<VerifiedContractSideEffectIntent> {
    if intent.intent.intent_schema_id
        == ContextContractDeployIntent::schema_id().map_err(replay_value_error)?
    {
        let deploy: ContextContractDeployIntent =
            serde_json::from_slice(&intent.artifact_bytes).map_err(replay_json_error)?;
        if prepared.transactions.len() != 1 {
            return Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "deploy intent transaction count does not match prepared invocation",
            ));
        }
        verify_transaction_intent_matches_prepared(
            &deploy.transaction,
            &prepared.transactions[0],
            prepared,
        )?;
        if prepared.phase != ContractMutationPhase::Deploy
            || prepared.resource_stage != ContractLifecycleStage::Deployed
            || deploy.transaction.to_address.is_some()
        {
            return Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "deploy intent does not match prepared invocation context",
            ));
        }
        return Ok(VerifiedContractSideEffectIntent::Deploy(Box::new(deploy)));
    }
    if intent.intent.intent_schema_id
        == ContextContractConfigureIntent::schema_id().map_err(replay_value_error)?
    {
        let configure: ContextContractConfigureIntent =
            serde_json::from_slice(&intent.artifact_bytes).map_err(replay_json_error)?;
        if prepared.phase != ContractMutationPhase::Configure
            || prepared.resource_stage != ContractLifecycleStage::Configured
            || configure.deployed.context_ref != prepared.context_ref
        {
            return Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "configure intent does not match prepared invocation context",
            ));
        }
        if configure.transactions.len() != prepared.transactions.len() {
            return Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "configure intent transaction count does not match prepared invocation",
            ));
        }
        for (intent_transaction, prepared_transaction) in configure
            .transactions
            .iter()
            .zip(prepared.transactions.iter())
        {
            verify_transaction_intent_matches_prepared(
                intent_transaction,
                prepared_transaction,
                prepared,
            )?;
        }
        return Ok(VerifiedContractSideEffectIntent::Configure(Box::new(
            configure,
        )));
    }
    Err(replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        "contract lifecycle side-effect intent schema did not match deploy or configure intent",
    ))
}

pub(crate) fn verify_replay_prepared_transaction_data_matches_certified_inputs(
    broker: &replay::ReplayBroker,
    submit_node: &spec::NodeSpec,
    intent: &VerifiedContractSideEffectIntent,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    let context = certified_evm_context_for_ref(broker, prepared.context_ref.as_context_ref())?;
    match intent {
        VerifiedContractSideEffectIntent::Deploy(intent) => {
            let action: DeployAction = replay_node_config(broker, submit_node)?;
            let state = ContextBoundDeployContractState::new(
                ValidatedConfig::new(action.clone()).map_err(replay_adapter_error)?,
            )
            .map_err(replay_adapter_error)?;
            let expected_intent = state
                .prepare_intent(&(), &context)
                .map_err(replay_adapter_error)?;
            if &expected_intent != intent.as_ref() {
                return Err(replay_contract_mismatch(
                    "deploy intent does not match certified node config",
                ));
            }
            let artifact = replay_context_profile_artifact(broker, &context)?;
            let tx_inputs = vec![PreparedTransactionInput {
                to: None,
                value_wei: parse_optional_wei(action.value_wei()).map_err(replay_adapter_error)?,
                data: deploy_action_data(&action, &artifact).map_err(replay_adapter_error)?,
            }];
            verify_prepared_transaction_data_matches_inputs(prepared, &tx_inputs)
        }
        VerifiedContractSideEffectIntent::Configure(intent) => {
            let action: ConfigureAction = replay_node_config(broker, submit_node)?;
            let deployed = configured_deployed_input_for_node(broker, submit_node, &context)?;
            let input = ContextConfigureContractInput { deployed };
            let state = ContextBoundConfigureContractState::new(
                ValidatedConfig::new(action.clone()).map_err(replay_adapter_error)?,
            )
            .map_err(replay_adapter_error)?;
            let expected_intent = state
                .prepare_intent(&input, &context)
                .map_err(replay_adapter_error)?;
            if &expected_intent != intent.as_ref() {
                return Err(replay_contract_mismatch(
                    "configure intent does not match certified node config and inputs",
                ));
            }
            let artifact = if configure_action_requires_artifact(&action) {
                Some(replay_context_profile_artifact(broker, &context)?)
            } else {
                None
            };
            let tx_inputs = configure_action_transaction_inputs(
                &action,
                artifact.as_ref(),
                input.deployed.address.as_str(),
            )
            .map_err(replay_adapter_error)?;
            verify_prepared_transaction_data_matches_inputs(prepared, &tx_inputs)
        }
    }
}

pub(crate) fn verify_prepared_transaction_data_matches_inputs(
    prepared: &PreparedContractInvocation,
    tx_inputs: &[PreparedTransactionInput],
) -> replay::Result<()> {
    if prepared.transactions.len() != tx_inputs.len() {
        return Err(replay_contract_mismatch(
            "prepared transaction count does not match certified transaction inputs",
        ));
    }
    for (index, (transaction, input)) in prepared.transactions.iter().zip(tx_inputs).enumerate() {
        let expected_to = input.to.as_ref().map(|address| format!("{address:?}"));
        if transaction.index != index as u64
            || transaction.chain_id != prepared.expected_chain_id
            || transaction.to_address != expected_to
            || transaction.value_wei != input.value_wei.to_string()
            || transaction.data_digest != digest_bytes(&input.data).to_string()
            || transaction.data_len != input.data.len() as u64
        {
            return Err(replay_contract_mismatch(
                "prepared transaction data does not match certified transaction inputs",
            ));
        }
    }
    Ok(())
}

pub(crate) fn verify_transaction_intent_matches_prepared(
    intent: &mfm_state_evm_contracts::ContextContractTransactionIntent,
    transaction: &PreparedContractTransactionEvidence,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    if intent.context_ref != prepared.context_ref
        || intent.network_id != prepared.network_id
        || intent.expected_chain_id != prepared.expected_chain_id
        || intent.signer_ref != prepared.signer_ref
        || normalize_address(&intent.expected_signer_address)
            .map_err(|error| replay_model_error(error.message))?
            != prepared.expected_signer_address
    {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "contract lifecycle intent context does not match prepared invocation",
        ));
    }
    let expected_to = intent
        .to_address
        .as_deref()
        .map(normalize_address)
        .transpose()
        .map_err(|error| replay_model_error(error.message))?;
    if transaction.to_address != expected_to
        || transaction.chain_id != intent.expected_chain_id
        || transaction.value_wei
            != parse_optional_wei(intent.value_wei.as_deref())
                .map_err(replay_adapter_error)?
                .to_string()
    {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "contract lifecycle transaction intent does not match prepared transaction",
        ));
    }
    verify_transaction_policy_matches_prepared(&intent.transaction, transaction)
}

pub(crate) fn verify_transaction_policy_matches_prepared(
    policy: &EvmTransactionPolicy,
    transaction: &PreparedContractTransactionEvidence,
) -> replay::Result<()> {
    let gas_limit = policy.gas_limit();
    if gas_limit.is_some_and(|gas_limit| gas_limit != transaction.gas_limit) {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "prepared transaction gas limit does not match certified policy",
        ));
    }
    match policy.style() {
        ConfigTransactionStyle::Eip1559 => {
            if transaction.style != PreparedContractTransactionStyle::Eip1559
                || transaction.gas_price.is_some()
            {
                return Err(replay::ReplayError::new(
                    replay::ReplayErrorKind::SideEffectMismatch,
                    "prepared transaction style does not match certified EIP-1559 policy",
                ));
            }
            verify_optional_policy_quantity_matches_prepared(
                policy.max_fee_per_gas(),
                transaction.max_fee_per_gas.as_deref(),
                "max_fee_per_gas",
            )?;
            verify_optional_policy_quantity_matches_prepared(
                policy.max_priority_fee_per_gas(),
                transaction.max_priority_fee_per_gas.as_deref(),
                "max_priority_fee_per_gas",
            )
        }
        ConfigTransactionStyle::Legacy => {
            if transaction.style != PreparedContractTransactionStyle::Legacy
                || transaction.max_fee_per_gas.is_some()
                || transaction.max_priority_fee_per_gas.is_some()
            {
                return Err(replay::ReplayError::new(
                    replay::ReplayErrorKind::SideEffectMismatch,
                    "prepared transaction style does not match certified legacy policy",
                ));
            }
            verify_optional_policy_quantity_matches_prepared(
                policy.gas_price(),
                transaction.gas_price.as_deref(),
                "gas_price",
            )
        }
    }
}

pub(crate) fn verify_optional_policy_quantity_matches_prepared(
    policy_value: Option<&str>,
    prepared_value: Option<&str>,
    field: &'static str,
) -> replay::Result<()> {
    let Some(policy_value) =
        optional_policy_quantity(policy_value, field).map_err(replay_adapter_error)?
    else {
        return Ok(());
    };
    let prepared_value =
        required_prepared_quantity(prepared_value, field).map_err(replay_adapter_error)?;
    if policy_value != prepared_value {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            format!("prepared transaction {field} does not match certified policy"),
        ));
    }
    Ok(())
}

pub(crate) fn verify_contract_receipt_schema(schema: &SchemaId) -> replay::Result<()> {
    let deploy = ContractDeployReceipt::schema_id().map_err(replay_value_error)?;
    let context_configure =
        ContextContractConfigureReceipt::schema_id().map_err(replay_value_error)?;
    if schema == &deploy || schema == &context_configure {
        Ok(())
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "receipt schema did not match contract lifecycle schemas",
        ))
    }
}

pub(crate) fn verify_contract_receipt_artifact(
    schema: &SchemaId,
    artifact_bytes: &[u8],
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    let deploy = ContractDeployReceipt::schema_id().map_err(replay_value_error)?;
    let context_configure =
        ContextContractConfigureReceipt::schema_id().map_err(replay_value_error)?;
    if schema == &deploy {
        let receipt: ContractDeployReceipt =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        verify_contract_deploy_receipt_matches_prepared(&receipt, prepared)
    } else if schema == &context_configure {
        let receipt: ContextContractConfigureReceipt =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        verify_contract_configure_receipt_matches_prepared(&receipt, prepared)
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "receipt schema did not match contract lifecycle schemas",
        ))
    }
}

pub(crate) fn verify_contract_confirmation_schema(
    schema: &SchemaId,
    artifact_bytes: &[u8],
    required_depth: u64,
) -> replay::Result<()> {
    let deploy = ContractDeployConfirmation::schema_id().map_err(replay_value_error)?;
    let context_configure =
        ContextContractConfigureConfirmation::schema_id().map_err(replay_value_error)?;
    if schema == &deploy {
        let confirmation: ContractDeployConfirmation =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        ensure_replay_confirmation_depth(confirmation.confirmations, required_depth)
    } else if schema == &context_configure {
        let confirmation: ContextContractConfigureConfirmation =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        ensure_replay_confirmation_depth(confirmation.confirmations, required_depth)
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "confirmation schema did not match contract lifecycle schemas",
        ))
    }
}

pub(crate) fn verify_contract_confirmation_artifact(
    schema: &SchemaId,
    artifact_bytes: &[u8],
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    let deploy = ContractDeployConfirmation::schema_id().map_err(replay_value_error)?;
    let context_configure =
        ContextContractConfigureConfirmation::schema_id().map_err(replay_value_error)?;
    if schema == &deploy {
        let confirmation: ContractDeployConfirmation =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        verify_contract_deploy_confirmation_matches_prepared(&confirmation, prepared)
    } else if schema == &context_configure {
        let confirmation: ContextContractConfigureConfirmation =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        verify_contract_configure_confirmation_matches_prepared(&confirmation, prepared)
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "confirmation schema did not match contract lifecycle schemas",
        ))
    }
}

pub(crate) fn verify_contract_deploy_receipt_matches_prepared(
    receipt: &ContractDeployReceipt,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    verify_prepared_context(
        prepared,
        ContractMutationPhase::Deploy,
        &receipt.context_ref,
        &receipt.evm_network_context_ref,
        receipt.resource_stage,
        ContractLifecycleStage::Deployed,
        "deploy receipt context does not match prepared invocation",
    )?;
    verify_transaction_receipts_match_prepared(prepared, std::slice::from_ref(&receipt.receipt))
}

pub(crate) fn verify_contract_configure_receipt_matches_prepared(
    receipt: &ContextContractConfigureReceipt,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    verify_prepared_context(
        prepared,
        ContractMutationPhase::Configure,
        &receipt.context_ref,
        &receipt.evm_network_context_ref,
        receipt.resource_stage,
        ContractLifecycleStage::Configured,
        "configure receipt context does not match prepared invocation",
    )?;
    verify_transaction_receipts_match_prepared(prepared, &receipt.receipts)
}

pub(crate) fn verify_contract_deploy_confirmation_matches_prepared(
    confirmation: &ContractDeployConfirmation,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    verify_prepared_context(
        prepared,
        ContractMutationPhase::Deploy,
        &confirmation.context_ref,
        &confirmation.evm_network_context_ref,
        confirmation.resource_stage,
        ContractLifecycleStage::Deployed,
        "deploy confirmation context does not match prepared invocation",
    )?;
    verify_transaction_receipts_match_prepared(
        prepared,
        std::slice::from_ref(&confirmation.receipt),
    )
}

pub(crate) fn verify_contract_configure_confirmation_matches_prepared(
    confirmation: &ContextContractConfigureConfirmation,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    verify_prepared_context(
        prepared,
        ContractMutationPhase::Configure,
        &confirmation.context_ref,
        &confirmation.evm_network_context_ref,
        confirmation.resource_stage,
        ContractLifecycleStage::Configured,
        "configure confirmation context does not match prepared invocation",
    )?;
    verify_transaction_receipts_match_prepared(prepared, &confirmation.receipts)
}

pub(crate) fn verify_prepared_context(
    prepared: &PreparedContractInvocation,
    expected_phase: ContractMutationPhase,
    actual_context_ref: &mfm_values::ContextRefValue,
    actual_evm_network_context_ref: &str,
    actual_resource_stage: ContractLifecycleStage,
    expected_resource_stage: ContractLifecycleStage,
    mismatch: &'static str,
) -> replay::Result<()> {
    if prepared.phase != expected_phase
        || actual_context_ref != &prepared.context_ref
        || actual_evm_network_context_ref != prepared.evm_network_context_ref
        || actual_resource_stage != expected_resource_stage
    {
        return Err(replay_contract_mismatch(mismatch));
    }
    Ok(())
}

pub(crate) fn verify_transaction_receipts_match_prepared(
    prepared: &PreparedContractInvocation,
    receipts: &[ContractTransactionReceipt],
) -> replay::Result<()> {
    if receipts.len() != prepared.transactions.len() {
        return Err(replay_contract_mismatch(
            "contract receipt count does not match prepared transactions",
        ));
    }
    for (prepared_transaction, receipt) in prepared.transactions.iter().zip(receipts) {
        if receipt.receipt_version != 1
            || receipt.context_ref != prepared.context_ref
            || receipt.evm_network_context_ref != prepared.evm_network_context_ref
            || receipt.resource_stage != prepared.resource_stage
            || !receipt.status
            || receipt.transaction_hash != prepared_transaction.expected_transaction_hash
        {
            return Err(replay_contract_mismatch(
                "contract receipt transaction evidence does not match prepared invocation",
            ));
        }
    }
    Ok(())
}
