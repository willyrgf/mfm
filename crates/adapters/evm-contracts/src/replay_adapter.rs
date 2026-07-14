use super::*;

#[path = "replay_prepared.rs"]
mod replay_prepared;
pub(super) use self::replay_prepared::*;

pub(super) struct EvmContractLifecycleReplayVerifier {
    verifier_id: events::ReplayVerifierId,
}

impl EvmContractLifecycleReplayVerifier {
    pub(super) fn new() -> Result<Self> {
        Ok(Self {
            verifier_id: replay_verifier_id()?,
        })
    }

    fn verify_adapter_binding(
        &self,
        intent: &replay::SideEffectIntentReplayEvidence,
    ) -> replay::Result<()> {
        let binding = evm_contract_lifecycle_adapter_binding().map_err(replay_adapter_error)?;
        if intent.intent.adapter_kind != *binding.adapter_kind()
            || intent.intent.adapter_version != *binding.adapter_version()
        {
            return Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "contract lifecycle adapter identity mismatch",
            ));
        }
        Ok(())
    }
}

impl Default for EvmContractLifecycleReplayVerifier {
    fn default() -> Self {
        Self::new().expect("valid verifier id")
    }
}

impl replay::SideEffectReplayVerifier for EvmContractLifecycleReplayVerifier {
    fn verifier_id(&self) -> &events::ReplayVerifierId {
        &self.verifier_id
    }

    fn verify_submission(
        &self,
        input: &replay::SideEffectSubmissionReplayInput,
    ) -> replay::Result<()> {
        self.verify_adapter_binding(&input.intent)?;
        ensure_schema(
            &input.submission.submission.submission_schema_id,
            &ContractTransactionSubmissions::schema_id().map_err(replay_value_error)?,
            "submission",
        )?;
        let prepared = replay_prepared_invocation(input.prepared_invocation.as_ref())?;
        verify_prepared_matches_certified_side_effect_context(&input.certified_context, &prepared)?;
        verify_replay_intent_matches_prepared(&input.intent, &prepared)?;
        let submissions: ContractTransactionSubmissions =
            serde_json::from_slice(&input.submission.artifact_bytes).map_err(replay_json_error)?;
        verify_prepared_submissions(&prepared, &submissions).map_err(replay_adapter_error)?;
        Ok(())
    }

    fn verify_receipt(&self, input: &replay::SideEffectReceiptReplayInput) -> replay::Result<()> {
        self.verify_adapter_binding(&input.intent)?;
        let prepared = replay_prepared_invocation(input.prepared_invocation.as_ref())?;
        verify_prepared_matches_certified_side_effect_context(&input.certified_context, &prepared)?;
        verify_replay_intent_matches_prepared(&input.intent, &prepared)?;
        if let Some(submission) = &input.submission {
            let submissions: ContractTransactionSubmissions =
                serde_json::from_slice(&submission.artifact_bytes).map_err(replay_json_error)?;
            verify_prepared_submissions(&prepared, &submissions).map_err(replay_adapter_error)?;
        }
        verify_contract_receipt_schema(&input.receipt.receipt.receipt_schema_id)?;
        verify_contract_receipt_artifact(
            &input.receipt.receipt.receipt_schema_id,
            &input.receipt.artifact_bytes,
            &prepared,
        )
    }

    fn verify_confirmation(
        &self,
        input: &replay::SideEffectConfirmationReplayInput,
    ) -> replay::Result<()> {
        self.verify_adapter_binding(&input.intent)?;
        let required_depth = match &input.verification {
            spec::SideEffectVerificationSpec::Finalized { depth } => *depth,
            spec::SideEffectVerificationSpec::Receipt => {
                return Err(replay::ReplayError::new(
                    replay::ReplayErrorKind::SideEffectMismatch,
                    "receipt-only contract lifecycle side effect recorded confirmation evidence",
                ));
            }
        };
        let prepared = replay_prepared_invocation(input.prepared_invocation.as_ref())?;
        verify_prepared_matches_certified_side_effect_context(&input.certified_context, &prepared)?;
        verify_replay_intent_matches_prepared(&input.intent, &prepared)?;
        if let Some(submission) = &input.submission {
            let submissions: ContractTransactionSubmissions =
                serde_json::from_slice(&submission.artifact_bytes).map_err(replay_json_error)?;
            verify_prepared_submissions(&prepared, &submissions).map_err(replay_adapter_error)?;
        }
        if let Some(receipt) = &input.receipt {
            verify_contract_receipt_artifact(
                &receipt.receipt.receipt_schema_id,
                &receipt.artifact_bytes,
                &prepared,
            )?;
        }
        verify_contract_confirmation_schema(
            &input.confirmation.confirmation.confirmation_schema_id,
            &input.confirmation.artifact_bytes,
            required_depth,
        )?;
        verify_contract_confirmation_artifact(
            &input.confirmation.confirmation.confirmation_schema_id,
            &input.confirmation.artifact_bytes,
            &prepared,
        )
    }
}

/// Returns the stable replay verifier id.
pub fn replay_verifier_id() -> Result<events::ReplayVerifierId> {
    events::ReplayVerifierId::new(REPLAY_VERIFIER_ID)
        .map_err(|error| EvmContractAdapterError::Identity(error.to_string()))
}

pub(super) fn contract_side_effect_replay_evidence() -> mfm_runtime::Result<SideEffectReplayEvidence>
{
    Ok(SideEffectReplayEvidence::new(replay_verifier_id()?, None))
}

/// Verifies contract lifecycle replay evidence for a dispatched lifecycle stream.
pub fn verify_contract_lifecycle_replay(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
) -> replay::Result<()> {
    let frames = broker.side_effect_replay_frames_matching(is_contract_lifecycle_replay_intent)?;
    let verifier = EvmContractLifecycleReplayVerifier::new()?;
    let mut verified_frames = Vec::with_capacity(frames.len());
    for frame in &frames {
        verified_frames.push(verify_contract_lifecycle_replay_frame(
            broker, &verifier, frame,
        )?);
    }
    verify_contract_lifecycle_state_outputs(broker, source_run_registry, &verified_frames)?;
    Ok(())
}

#[derive(Debug, Clone)]
struct VerifiedContractSideEffectFrame {
    pair_id: mfm_ids::SideEffectPairId,
    node_id: NodeId,
    intent: VerifiedContractSideEffectIntent,
    prepared: PreparedContractInvocation,
    terminal: VerifiedContractSideEffectTerminal,
}

#[derive(Debug, Clone)]
enum VerifiedContractSideEffectTerminal {
    DeployReceipt(ContractDeployReceipt),
    DeployConfirmation(ContractDeployConfirmation),
    ConfigureReceipt(ContextContractConfigureReceipt),
    ConfigureConfirmation(ContextContractConfigureConfirmation),
}

fn verify_contract_lifecycle_replay_frame(
    broker: &replay::ReplayBroker,
    verifier: &EvmContractLifecycleReplayVerifier,
    frame: &replay::SideEffectReplayFrame<'_>,
) -> replay::Result<VerifiedContractSideEffectFrame> {
    let pair = broker
        .certified_spec()
        .spec
        .side_effect_verify_pair_for_pair_id(&frame.intent.pair_id)
        .map_err(replay_adapter_error)?;
    let Some(submission_request) = frame.submission_request() else {
        return Err(contract_lifecycle_side_effect_missing("submission"));
    };
    let Some(receipt_request) = frame.receipt_request() else {
        return Err(contract_lifecycle_side_effect_missing("receipt"));
    };

    broker.verify_side_effect_submission(&submission_request, verifier)?;
    let receipt = broker.verify_side_effect_receipt(&receipt_request, verifier)?;
    let receipt_terminal = decode_verified_contract_receipt(&receipt)?;
    let terminal = if let Some(confirmation_request) = frame.confirmation_request() {
        let confirmation =
            broker.verify_side_effect_confirmation(&confirmation_request, verifier)?;
        decode_verified_contract_confirmation(&confirmation)?
    } else {
        receipt_terminal
    };
    let prepared = broker.side_effect_prepared_invocation(&receipt_request)?;
    let prepared = replay_prepared_invocation(Some(&prepared))?;
    let intent_evidence = replay_side_effect_intent(broker, frame.intent)?;
    let intent = verify_replay_intent_matches_prepared(&intent_evidence, &prepared)?;
    verify_replay_prepared_transaction_data_matches_certified_inputs(
        broker,
        pair.submit_node,
        &intent,
        &prepared,
    )?;
    Ok(VerifiedContractSideEffectFrame {
        pair_id: frame.intent.pair_id.clone(),
        node_id: frame.intent.node_id.clone(),
        intent,
        prepared,
        terminal,
    })
}

fn replay_side_effect_intent(
    broker: &replay::ReplayBroker,
    intent: &events::side_effect::IntentPersisted,
) -> replay::Result<replay::SideEffectIntentReplayEvidence> {
    let artifact = broker.retained_artifact(&store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::SideEffectIntent,
        artifact_id: intent.intent_artifact_id.clone(),
        evidence_hash: intent.intent_artifact_evidence_hash.clone(),
        digest: Some(intent.intent_hash.clone()),
        byte_len: None,
        media_type: None,
        schema_id: Some(intent.intent_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(intent.node_id.clone()),
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::SideEffectIntent),
    })?;
    Ok(replay::SideEffectIntentReplayEvidence {
        intent: intent.clone(),
        artifact: artifact.artifact,
        artifact_bytes: artifact.artifact_bytes,
    })
}

fn decode_verified_contract_receipt(
    receipt: &replay::ReceiptReplayEvidence,
) -> replay::Result<VerifiedContractSideEffectTerminal> {
    let deploy = ContractDeployReceipt::schema_id().map_err(replay_value_error)?;
    let configure = ContextContractConfigureReceipt::schema_id().map_err(replay_value_error)?;
    if receipt.receipt.receipt_schema_id == deploy {
        serde_json::from_slice(&receipt.artifact_bytes)
            .map(VerifiedContractSideEffectTerminal::DeployReceipt)
            .map_err(replay_json_error)
    } else if receipt.receipt.receipt_schema_id == configure {
        serde_json::from_slice(&receipt.artifact_bytes)
            .map(VerifiedContractSideEffectTerminal::ConfigureReceipt)
            .map_err(replay_json_error)
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "receipt schema did not match contract lifecycle schemas",
        ))
    }
}

fn decode_verified_contract_confirmation(
    confirmation: &replay::ConfirmationReplayEvidence,
) -> replay::Result<VerifiedContractSideEffectTerminal> {
    let deploy = ContractDeployConfirmation::schema_id().map_err(replay_value_error)?;
    let configure =
        ContextContractConfigureConfirmation::schema_id().map_err(replay_value_error)?;
    if confirmation.confirmation.confirmation_schema_id == deploy {
        serde_json::from_slice(&confirmation.artifact_bytes)
            .map(VerifiedContractSideEffectTerminal::DeployConfirmation)
            .map_err(replay_json_error)
    } else if confirmation.confirmation.confirmation_schema_id == configure {
        serde_json::from_slice(&confirmation.artifact_bytes)
            .map(VerifiedContractSideEffectTerminal::ConfigureConfirmation)
            .map_err(replay_json_error)
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "confirmation schema did not match contract lifecycle schemas",
        ))
    }
}

fn verify_contract_lifecycle_state_outputs(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
    verified_frames: &[VerifiedContractSideEffectFrame],
) -> replay::Result<usize> {
    let frames = broker.produced_cell_frames_matching(is_contract_lifecycle_output_cell)?;
    for frame in &frames {
        verify_contract_lifecycle_state_output(
            broker,
            source_run_registry,
            verified_frames,
            frame,
        )?;
    }
    Ok(frames.len())
}

fn is_contract_lifecycle_output_cell(
    _node: &spec::NodeSpec,
    cell: &spec::CellSpec,
    produced: &events::CellProduced,
) -> replay::Result<bool> {
    let deployed_schema = DeployedContractInstance::schema_id().map_err(replay_value_error)?;
    let configured_schema = ConfiguredContractInstance::schema_id().map_err(replay_value_error)?;
    let report_schema = ContextBoundValidationReport::schema_id().map_err(replay_value_error)?;
    if produced.schema_id == deployed_schema
        || produced.schema_id == configured_schema
        || produced.schema_id == report_schema
    {
        return Ok(true);
    }
    Ok(matches!(
        &cell.context,
        spec::CellContextSpec::Bound { resource_kind, .. }
            if resource_kind == contract_instance_resource_kind()
                || resource_kind == validation_report_resource_kind()
    ))
}

fn verify_contract_lifecycle_state_output(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
    verified_frames: &[VerifiedContractSideEffectFrame],
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let deployed_schema = DeployedContractInstance::schema_id().map_err(replay_value_error)?;
    let configured_schema = ConfiguredContractInstance::schema_id().map_err(replay_value_error)?;
    let report_schema = ContextBoundValidationReport::schema_id().map_err(replay_value_error)?;
    if frame.produced.schema_id == deployed_schema {
        let output: DeployedContractInstance =
            serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
        verify_deployed_replay_output(broker, source_run_registry, verified_frames, frame, &output)
    } else if frame.produced.schema_id == configured_schema {
        let output: ConfiguredContractInstance =
            serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
        verify_configured_replay_output(
            broker,
            source_run_registry,
            verified_frames,
            frame,
            &output,
        )
    } else if frame.produced.schema_id == report_schema {
        let output: ContextBoundValidationReport =
            serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
        verify_validation_report_replay_output(broker, frame, &output)
    } else {
        Err(replay_contract_mismatch(
            "contract lifecycle output schema is not a registered lifecycle value",
        ))
    }
}

fn verify_deployed_replay_output(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
    verified_frames: &[VerifiedContractSideEffectFrame],
    frame: &replay::ProducedCellReplayFrame,
    output: &DeployedContractInstance,
) -> replay::Result<()> {
    let context = verify_context_bound_replay_output::<DeployedContractInstance>(
        broker,
        frame,
        output,
        contract_instance_resource_kind(),
        deployed_contract_stage(),
    )?;
    match &output.deploy_provenance {
        DeployProvenance::MfmDeploy { .. } => {
            let expected = expected_deployed_output_from_verified_side_effect(
                broker,
                frame,
                verified_frames,
                &context,
            )?;
            if &expected != output {
                return Err(replay_contract_mismatch(
                    "mfm deploy output does not match replayed side-effect evidence",
                ));
            }
            if output.external_adoption_evidence.is_some() {
                return Err(replay_contract_mismatch(
                    "mfm deploy output carried external adoption evidence",
                ));
            }
            Ok(())
        }
        DeployProvenance::ImportedMfmRun {
            source_context_ref,
            source_value_digest,
            import_policy_digest,
        } => {
            let import: ImportDeployedSpec = replay_node_config(broker, &frame.node)?;
            let ImportDeployedSpec::FromMfmRun { source, evidence } = &import else {
                return Err(replay_contract_mismatch(
                    "deployed import output was not produced from source-run import config",
                ));
            };
            if source_context_ref != &evidence.source_context_ref
                || source_value_digest != &evidence.source_value_digest
                || import_policy_digest != &evidence.import_policy_digest
                || output.deploy_evidence != source_run_import_evidence_refs(evidence)
            {
                return Err(replay_contract_mismatch(
                    "deployed source-run import output does not match retained import evidence",
                ));
            }
            let imported = verify_replayed_source_run_import::<DeployedContractInstance>(
                broker,
                source_run_registry,
                source,
                evidence,
                &context,
                ContractLifecycleStage::Deployed,
            )?;
            let expected = ImportDeployedContractState::admit_verified_mfm_run_import(
                &import, imported, &context,
            )
            .map_err(replay_adapter_error)?;
            if &expected != output {
                return Err(replay_contract_mismatch(
                    "deployed source-run import output does not match replayed projection",
                ));
            }
            if output.external_adoption_evidence.is_some() {
                return Err(replay_contract_mismatch(
                    "source-run deployed import carried external adoption evidence",
                ));
            }
            Ok(())
        }
        DeployProvenance::ExternalAdoption {
            evidence_policy_digest,
            ..
        } => {
            let import: ImportDeployedSpec = replay_node_config(broker, &frame.node)?;
            let ImportDeployedSpec::AdoptExternalAddress { adoption } = &import else {
                return Err(replay_contract_mismatch(
                    "deployed external adoption output was not produced from adoption config",
                ));
            };
            let evidence = output.external_adoption_evidence.as_ref().ok_or_else(|| {
                replay_contract_mismatch("deployed external adoption evidence is missing")
            })?;
            if evidence_policy_digest != &evidence.evidence_policy_digest
                || !output.deploy_evidence.is_empty()
            {
                return Err(replay_contract_mismatch(
                    "deployed external adoption output evidence does not match provenance",
                ));
            }
            verify_replayed_external_adoption(
                evidence,
                &context,
                ContractLifecycleStage::Deployed,
                adoption,
            )
        }
    }
}

fn verify_configured_replay_output(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
    verified_frames: &[VerifiedContractSideEffectFrame],
    frame: &replay::ProducedCellReplayFrame,
    output: &ConfiguredContractInstance,
) -> replay::Result<()> {
    let context = verify_context_bound_replay_output::<ConfiguredContractInstance>(
        broker,
        frame,
        output,
        contract_instance_resource_kind(),
        configured_contract_stage(),
    )?;
    if output.configured_from.deployed_context_ref.as_context_ref() != context.context_ref() {
        return Err(replay_contract_mismatch(
            "configured output source deployed context does not match certified context",
        ));
    }
    match &output.configuration_claim {
        ConfigurationClaim::MfmConfigured { .. } => {
            let expected = expected_configured_output_from_verified_side_effect(
                broker,
                frame,
                verified_frames,
                &context,
            )?;
            if &expected != output {
                return Err(replay_contract_mismatch(
                    "mfm configured output does not match replayed side-effect evidence",
                ));
            }
            if output.external_adoption_evidence.is_some() {
                return Err(replay_contract_mismatch(
                    "mfm configured output carried external adoption evidence",
                ));
            }
        }
        ConfigurationClaim::ImportedMfmConfigured {
            source_run_id,
            source_spec_hash,
            source_cell_or_output_id,
            source_value_digest,
            source_context_ref,
        } => {
            let import: ImportConfiguredSpec = replay_node_config(broker, &frame.node)?;
            let ImportConfiguredSpec::FromMfmRun { source, evidence } = &import else {
                return Err(replay_contract_mismatch(
                    "configured import output was not produced from source-run import config",
                ));
            };
            if source_run_id != &source.source_run_id
                || source_spec_hash != &evidence.source_spec_hash
                || source_cell_or_output_id != &evidence.source_cell_or_output_id
                || source_value_digest != &evidence.source_value_digest
                || source_context_ref != &evidence.source_context_ref
                || output.configure_or_import_evidence != source_run_import_evidence_refs(evidence)
            {
                return Err(replay_contract_mismatch(
                    "configured source-run import output does not match retained import evidence",
                ));
            }
            let imported = verify_replayed_source_run_import::<ConfiguredContractInstance>(
                broker,
                source_run_registry,
                source,
                evidence,
                &context,
                ContractLifecycleStage::Configured,
            )?;
            let expected = ImportConfiguredContractState::admit_verified_mfm_run_import(
                &import, imported, &context,
            )
            .map_err(replay_adapter_error)?;
            if &expected != output {
                return Err(replay_contract_mismatch(
                    "configured source-run import output does not match replayed projection",
                ));
            }
            if output.external_adoption_evidence.is_some() {
                return Err(replay_contract_mismatch(
                    "source-run configured import carried external adoption evidence",
                ));
            }
        }
        ConfigurationClaim::ExternalObservedConfigured {
            evidence_policy_digest,
            external_adoption_evidence_digest,
            ..
        } => {
            let import: ImportConfiguredSpec = replay_node_config(broker, &frame.node)?;
            let ImportConfiguredSpec::AdoptExternalAddress { adoption } = &import else {
                return Err(replay_contract_mismatch(
                    "configured external adoption output was not produced from adoption config",
                ));
            };
            let evidence = output.external_adoption_evidence.as_ref().ok_or_else(|| {
                replay_contract_mismatch("configured external adoption evidence is missing")
            })?;
            if evidence_policy_digest != &evidence.evidence_policy_digest
                || !output.configure_or_import_evidence.is_empty()
            {
                return Err(replay_contract_mismatch(
                    "configured external adoption output evidence does not match provenance",
                ));
            }
            if digest_for_value(evidence).map_err(replay_adapter_error)?
                != *external_adoption_evidence_digest
            {
                return Err(replay_contract_mismatch(
                    "configured external adoption evidence digest does not match claim",
                ));
            }
            verify_replayed_external_adoption(
                evidence,
                &context,
                ContractLifecycleStage::Configured,
                adoption,
            )?;
        }
        ConfigurationClaim::ExternalClaimedConfigured {
            evidence_policy_digest,
            ..
        } => {
            let import: ImportConfiguredSpec = replay_node_config(broker, &frame.node)?;
            let ImportConfiguredSpec::AdoptExternalAddress { adoption } = &import else {
                return Err(replay_contract_mismatch(
                    "configured external adoption output was not produced from adoption config",
                ));
            };
            let evidence = output.external_adoption_evidence.as_ref().ok_or_else(|| {
                replay_contract_mismatch("configured external adoption evidence is missing")
            })?;
            if evidence_policy_digest != &evidence.evidence_policy_digest
                || !output.configure_or_import_evidence.is_empty()
            {
                return Err(replay_contract_mismatch(
                    "configured external adoption output evidence does not match provenance",
                ));
            }
            verify_replayed_external_adoption(
                evidence,
                &context,
                ContractLifecycleStage::Configured,
                adoption,
            )?;
        }
    }
    Ok(())
}

fn expected_deployed_output_from_verified_side_effect(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
    verified_frames: &[VerifiedContractSideEffectFrame],
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> replay::Result<DeployedContractInstance> {
    let pair = side_effect_pair_for_output(broker, frame)?;
    let side_effect = verified_side_effect_for_pair(&pair, verified_frames)?;
    let VerifiedContractSideEffectIntent::Deploy(intent) = &side_effect.intent else {
        return Err(replay_contract_mismatch(
            "deploy output was bound to non-deploy side-effect intent",
        ));
    };
    if side_effect.prepared.phase != ContractMutationPhase::Deploy {
        return Err(replay_contract_mismatch(
            "deploy output was bound to non-deploy prepared invocation",
        ));
    }
    let action: DeployAction = replay_node_config(broker, pair.submit_node)?;
    let state = ContextBoundDeployContractState::new(
        ValidatedConfig::new(action).map_err(replay_adapter_error)?,
    )
    .map_err(replay_adapter_error)?;
    match &side_effect.terminal {
        VerifiedContractSideEffectTerminal::DeployReceipt(receipt) => state
            .output_from_receipt(&(), intent, receipt, context)
            .map_err(replay_adapter_error),
        VerifiedContractSideEffectTerminal::DeployConfirmation(confirmation) => state
            .output_from_confirmation(&(), intent, confirmation, context)
            .map_err(replay_adapter_error),
        _ => Err(replay_contract_mismatch(
            "deploy output was bound to non-deploy terminal side-effect evidence",
        )),
    }
}

fn expected_configured_output_from_verified_side_effect(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
    verified_frames: &[VerifiedContractSideEffectFrame],
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> replay::Result<ConfiguredContractInstance> {
    let pair = side_effect_pair_for_output(broker, frame)?;
    let side_effect = verified_side_effect_for_pair(&pair, verified_frames)?;
    let VerifiedContractSideEffectIntent::Configure(intent) = &side_effect.intent else {
        return Err(replay_contract_mismatch(
            "configured output was bound to non-configure side-effect intent",
        ));
    };
    if side_effect.prepared.phase != ContractMutationPhase::Configure {
        return Err(replay_contract_mismatch(
            "configured output was bound to non-configure prepared invocation",
        ));
    }
    let deployed = configured_deployed_input_for_node(broker, pair.submit_node, context)?;
    let action: ConfigureAction = replay_node_config(broker, pair.submit_node)?;
    let state = ContextBoundConfigureContractState::new(
        ValidatedConfig::new(action).map_err(replay_adapter_error)?,
    )
    .map_err(replay_adapter_error)?;
    let input = ContextConfigureContractInput { deployed };
    match &side_effect.terminal {
        VerifiedContractSideEffectTerminal::ConfigureReceipt(receipt) => state
            .output_from_receipt(&input, intent, receipt, context)
            .map_err(replay_adapter_error),
        VerifiedContractSideEffectTerminal::ConfigureConfirmation(confirmation) => state
            .output_from_confirmation(&input, intent, confirmation, context)
            .map_err(replay_adapter_error),
        _ => Err(replay_contract_mismatch(
            "configured output was bound to non-configure terminal side-effect evidence",
        )),
    }
}

fn side_effect_pair_for_output<'a>(
    broker: &'a replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<spec::SideEffectVerifyPairRef<'a>> {
    broker
        .certified_spec()
        .spec
        .side_effect_verify_pair_for_verify_node(&frame.node.node_id)
        .map_err(replay_adapter_error)
}

fn verified_side_effect_for_pair<'a>(
    pair: &spec::SideEffectVerifyPairRef<'_>,
    verified_frames: &'a [VerifiedContractSideEffectFrame],
) -> replay::Result<&'a VerifiedContractSideEffectFrame> {
    verified_frames
        .iter()
        .find(|verified| {
            verified.pair_id == *pair.pair_id && verified.node_id == pair.submit_node.node_id
        })
        .ok_or_else(|| {
            replay_contract_mismatch(
                "contract lifecycle output lacks verified side-effect evidence for its certified pair",
            )
        })
}

fn configured_deployed_input_for_node(
    broker: &replay::ReplayBroker,
    node: &spec::NodeSpec,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> replay::Result<DeployedContractInstance> {
    let deployed = replay_input_cell_value::<DeployedContractInstance>(
        broker,
        node,
        contract_instance_resource_kind(),
        deployed_contract_stage(),
    )?;
    if deployed.context_ref.as_context_ref() != context.context_ref() {
        return Err(replay_contract_mismatch(
            "configured output deployed input context does not match certified context",
        ));
    }
    Ok(deployed)
}

fn replay_input_cell_value<T>(
    broker: &replay::ReplayBroker,
    node: &spec::NodeSpec,
    expected_resource_kind: &mfm_ids::ContextResourceKind,
    expected_stage: &mfm_ids::ContextStage,
) -> replay::Result<T>
where
    T: ContextBoundOutput + MfmValue + DeserializeOwned,
{
    let schema = T::schema_id().map_err(replay_value_error)?;
    let semantic = T::semantic_id().map_err(replay_value_error)?;
    let cell = unique_input_cell(&node.input_bindings.root, &schema, &semantic)?;
    let frames = broker.produced_cell_frames_matching(|_node, _cell, produced| {
        Ok(produced.cell_id == cell.cell_id)
    })?;
    let mut frames = frames.into_iter();
    let frame = frames.next().ok_or_else(|| {
        replay_contract_mismatch("certified input cell was not produced in replay stream")
    })?;
    if frames.next().is_some() {
        return Err(replay_contract_mismatch(
            "certified input cell has multiple produced replay frames",
        ));
    }
    let value: T = serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
    verify_context_bound_replay_output(
        broker,
        &frame,
        &value,
        expected_resource_kind,
        expected_stage,
    )?;
    Ok(value)
}

fn unique_input_cell<'a>(
    root: &'a spec::InputBindingNodeSpec,
    schema: &SchemaId,
    semantic: &mfm_ids::SemanticTypeId,
) -> replay::Result<&'a spec::InputBindingCellSpec> {
    let mut matches = Vec::new();
    collect_input_cells(root, schema, semantic, &mut matches);
    if matches.len() != 1 {
        return Err(replay_contract_mismatch(
            "certified node input bindings do not contain exactly one required lifecycle input cell",
        ));
    }
    Ok(matches.remove(0))
}

fn collect_input_cells<'a>(
    node: &'a spec::InputBindingNodeSpec,
    schema: &SchemaId,
    semantic: &mfm_ids::SemanticTypeId,
    matches: &mut Vec<&'a spec::InputBindingCellSpec>,
) {
    match node {
        spec::InputBindingNodeSpec::Unit => {}
        spec::InputBindingNodeSpec::Cell(cell) => {
            if cell.schema_id == *schema && cell.semantic_type_id == *semantic {
                matches.push(cell);
            }
        }
        spec::InputBindingNodeSpec::Tuple(elements)
        | spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells(element, schema, semantic, matches);
            }
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                collect_input_cells(&field.node, schema, semantic, matches);
            }
        }
    }
}

fn verify_validation_report_replay_output(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
    output: &ContextBoundValidationReport,
) -> replay::Result<()> {
    let context = verify_context_bound_replay_output::<ContextBoundValidationReport>(
        broker,
        frame,
        output,
        validation_report_resource_kind(),
        validation_report_stage(),
    )?;
    if output.configured_instance.context_ref.as_context_ref() != context.context_ref() {
        return Err(replay_contract_mismatch(
            "validation report configured instance context does not match certified context",
        ));
    }
    let configured_input = replay_input_cell_value::<ConfiguredContractInstance>(
        broker,
        &frame.node,
        contract_instance_resource_kind(),
        configured_contract_stage(),
    )?;
    if output.configured_instance
        != ConfiguredContractInstanceRef::from_configured(&configured_input)
    {
        return Err(replay_contract_mismatch(
            "validation report configured instance does not match certified input cell",
        ));
    }
    if output.evidence_refs != configured_input.configure_or_import_evidence {
        return Err(replay_contract_mismatch(
            "validation report evidence refs do not match certified configured input",
        ));
    }
    for evidence in &output.evidence_refs {
        replay_lifecycle_artifact(broker, evidence)?;
    }
    let action: ValidateAction = replay_node_config(broker, &frame.node)?;
    verify_validation_results_match_action(output, &action)?;
    verify_validation_source_evidence(
        &context,
        &output.read_results,
        &output.validation_read_evidence,
        &output.event_results,
        &output.validation_event_evidence,
    )?;
    for result in output
        .configuration_read_results
        .iter()
        .chain(output.read_results.iter())
    {
        require_validation_read_result_canonical_passed(result).map_err(replay_adapter_error)?;
    }
    for result in output
        .configuration_event_results
        .iter()
        .chain(output.event_results.iter())
    {
        require_validation_event_result_canonical_passed(result).map_err(replay_adapter_error)?;
    }
    let valid = output.observed_chain_id == context.value().network.expected_chain_id()
        && output
            .configuration_read_results
            .iter()
            .all(validation_read_result_passes)
        && output
            .configuration_event_results
            .iter()
            .all(validation_event_result_passes)
        && output
            .read_results
            .iter()
            .all(validation_read_result_passes)
        && output
            .event_results
            .iter()
            .all(validation_event_result_passes);
    if output.valid != valid {
        return Err(replay_contract_mismatch(
            "validation report validity does not match retained evidence",
        ));
    }
    Ok(())
}

fn verify_context_bound_replay_output<T>(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
    output: &T,
    expected_resource_kind: &mfm_ids::ContextResourceKind,
    expected_stage: &mfm_ids::ContextStage,
) -> replay::Result<mfm_program::CertifiedContext<EvmContractContext>>
where
    T: ContextBoundOutput + MfmValue,
{
    let expected_schema = T::schema_id().map_err(replay_value_error)?;
    let expected_semantic = T::semantic_id().map_err(replay_value_error)?;
    let (
        spec::NodeContextSpec::Required { context_ref },
        spec::CellContextSpec::Bound {
            context_ref: output_context_ref,
            resource_kind,
            stage,
            ..
        },
    ) = (&frame.node.context, &frame.cell.context)
    else {
        return Err(replay_contract_mismatch(
            "contract lifecycle output is missing certified context authority",
        ));
    };
    if context_ref != output_context_ref
        || resource_kind != expected_resource_kind
        || stage != expected_stage
        || output.context_ref() != context_ref
        || output.context_resource_kind() != expected_resource_kind
        || output.context_stage() != expected_stage
        || frame.produced.context != frame.cell.context
        || frame.produced.schema_id != expected_schema
        || frame.produced.semantic_type_id != expected_semantic
    {
        return Err(replay_contract_mismatch(
            "contract lifecycle output context does not match certified cell authority",
        ));
    }
    certified_evm_context_for_ref(broker, context_ref)
}

fn certified_evm_context_for_ref(
    broker: &replay::ReplayBroker,
    context_ref: &mfm_ids::ContextRef,
) -> replay::Result<mfm_program::CertifiedContext<EvmContractContext>> {
    let context = broker
        .certified_spec()
        .spec
        .contexts
        .iter()
        .find(|context| &context.context_ref == context_ref)
        .ok_or_else(|| replay_contract_mismatch("certified context table entry is missing"))?;
    mfm_program::CertifiedContext::<EvmContractContext>::from_certified_spec(context)
        .map_err(replay_adapter_error)
}

fn replay_node_config<T>(broker: &replay::ReplayBroker, node: &spec::NodeSpec) -> replay::Result<T>
where
    T: MfmConfig + DeserializeOwned,
{
    let config_evidence = store::ArtifactEvidenceRef {
        artifact_id: node.config_ref.artifact_id.clone(),
        digest: node.config_ref.digest.clone(),
        byte_len: node.config_ref.byte_len,
        media_type: node.config_ref.media_type.clone(),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::RunConfig,
        artifact_id: node.config_ref.artifact_id.clone(),
        evidence_hash: config_evidence
            .evidence_hash()
            .map_err(replay_adapter_error)?,
        digest: Some(node.config_ref.digest.clone()),
        byte_len: Some(node.config_ref.byte_len),
        media_type: Some(node.config_ref.media_type.clone()),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    };
    let artifact = broker.retained_artifact(&requirement)?;
    let config: T = serde_json::from_slice(&artifact.artifact_bytes).map_err(replay_json_error)?;
    ValidatedConfig::new(config)
        .map(ValidatedConfig::into_inner)
        .map_err(replay_adapter_error)
}

fn verify_replayed_source_run_import<T>(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
    source: &ImportFromMfmRun,
    evidence: &ImportFromMfmRunEvidence,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
) -> replay::Result<T>
where
    T: ContextBoundOutput + DeserializeOwned + Serialize,
{
    validate_source_run_import_policy(source, context, required_stage)
        .map_err(replay_adapter_error)?;
    validate_source_run_import_evidence::<T>(source, evidence, context, required_stage)
        .map_err(replay_adapter_error)?;
    let certificate = replay_lifecycle_artifact(broker, &evidence.source_spec_certificate_ref)?;
    let bundle_artifact = replay_lifecycle_artifact(broker, &evidence.source_run_stream_ref)?;
    let source_value_artifact = replay_lifecycle_artifact(
        broker,
        &evidence.source_value_artifact_ref_or_inline_canonical_value,
    )?;
    let source_value: T =
        serde_json::from_slice(&source_value_artifact.artifact_bytes).map_err(replay_json_error)?;
    if source_value.context_ref() != evidence.source_context_ref.as_context_ref()
        || source_value.context_resource_kind() != contract_instance_resource_kind()
        || source_value.context_stage() != context_stage_for_lifecycle_stage(required_stage)
        || digest_for_value(&source_value).map_err(replay_adapter_error)?
            != evidence.source_value_digest
    {
        return Err(replay_contract_mismatch(
            "source-run import source value does not match retained evidence",
        ));
    }
    let committed = decode_source_run_committed_stream(
        source,
        &bundle_artifact.artifact_bytes,
        &source_value_artifact.artifact,
        &source_value_artifact.artifact_bytes,
    )
    .map_err(replay_adapter_error)?;
    let run_admitted = source_run_admitted(&committed).map_err(replay_adapter_error)?;
    let source_spec = replay_source_run_artifact(
        broker,
        events::EventArtifactReferenceSource::RunSpec,
        &run_admitted.spec_artifact,
    )?;
    validate_source_run_authority(SourceRunAuthorityEvidence {
        source,
        evidence,
        source_spec_bytes: &source_spec.artifact_bytes,
        certificate_bytes: &certificate.artifact_bytes,
        committed: &committed,
        source_value_artifact: &source_value_artifact.artifact,
        required_stage,
        source_run_registry,
    })
    .map_err(replay_adapter_error)?;
    Ok(source_value)
}

fn replay_lifecycle_artifact(
    broker: &replay::ReplayBroker,
    evidence: &LifecycleArtifactEvidenceRef,
) -> replay::Result<replay::ArtifactReplayEvidence> {
    let requirement = lifecycle_artifact_requirement(evidence).map_err(replay_adapter_error)?;
    broker.retained_artifact(&requirement)
}

fn replay_context_profile_artifact(
    broker: &replay::ReplayBroker,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> replay::Result<ContractArtifactConfig> {
    let reference = context
        .value()
        .contract_profile
        .artifact_ref
        .as_ref()
        .ok_or_else(|| replay_adapter_error(EvmContractAdapterError::MissingContractArtifact))?;
    let requirement =
        contract_profile_artifact_requirement(reference).map_err(replay_adapter_error)?;
    let artifact = broker.retained_artifact(&requirement)?;
    serde_json::from_slice::<ContractArtifactConfig>(&artifact.artifact_bytes)
        .map_err(replay_json_error)
}

fn replay_source_run_artifact(
    broker: &replay::ReplayBroker,
    source: events::EventArtifactReferenceSource,
    artifact: &events::RunArtifactEvidenceRef,
) -> replay::Result<replay::ArtifactReplayEvidence> {
    broker.retained_artifact(&run_artifact_requirement(source, artifact))
}

fn source_run_import_evidence_refs(
    evidence: &ImportFromMfmRunEvidence,
) -> Vec<LifecycleArtifactEvidenceRef> {
    vec![
        evidence.source_spec_artifact_ref.clone(),
        evidence.source_spec_certificate_ref.clone(),
        evidence.source_run_stream_ref.clone(),
        evidence
            .source_value_artifact_ref_or_inline_canonical_value
            .clone(),
    ]
}

pub(super) fn verify_replayed_external_adoption(
    evidence: &ExternalAdoptionEvidence,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
    adoption: &mfm_evm_contract_model::AdoptExternalAddress,
) -> replay::Result<()> {
    let policy = &adoption.evidence_policy;
    if evidence.context_ref.as_context_ref() != context.context_ref()
        || evidence.evm_network_context_ref
            != evm_network_context_ref(&context.value().network).map_err(replay_adapter_error)?
        || evidence.resource_stage != required_stage
        || evidence.observed_chain_id != context.value().network.expected_chain_id()
        || evidence.evidence_policy_digest
            != digest_for_value(policy).map_err(replay_adapter_error)?
    {
        return Err(replay_contract_mismatch(
            "external adoption evidence context does not match certified import authority",
        ));
    }
    if policy.require_code || policy.expected_code_hash.is_some() {
        let code = evidence.code_read_evidence.as_ref().ok_or_else(|| {
            replay_contract_mismatch("external adoption code-read evidence is missing")
        })?;
        verify_external_source_evidence(context, &code.source)?;
        let expected_block = policy
            .block_anchor
            .clone()
            .unwrap_or(ModelBlockSelector::Tag {
                tag: BlockTag::Latest,
            });
        if code.address != adoption.address
            || code.block != expected_block
            || (policy.require_code && code.observed_code_byte_len == 0)
            || policy
                .expected_code_hash
                .as_ref()
                .is_some_and(|expected| expected != &code.observed_code_hash)
        {
            return Err(replay_contract_mismatch(
                "external adoption code-read evidence does not match certified policy",
            ));
        }
    } else if evidence.code_read_evidence.is_some() {
        return Err(replay_contract_mismatch(
            "external adoption carried code-read evidence outside certified policy",
        ));
    }
    if evidence.read_assertion_evidence.len() != policy.initial_read_assertions.len()
        || evidence.event_assertion_evidence.len() != policy.initial_event_assertions.len()
    {
        return Err(replay_contract_mismatch(
            "external adoption assertion evidence count does not match certified policy",
        ));
    }
    for (record, assertion) in evidence
        .read_assertion_evidence
        .iter()
        .zip(policy.initial_read_assertions.iter())
    {
        verify_external_source_evidence(context, &record.source)?;
        if record.result.function.as_str() != assertion.function.as_str()
            || record.result.args != assertion.args
            || record.result.expected != assertion.expected
        {
            return Err(replay_contract_mismatch(
                "external adoption read assertion evidence does not match certified policy",
            ));
        }
        require_validation_read_result_canonical_passed(&record.result)
            .map_err(replay_adapter_error)?;
        if !validation_read_result_passes(&record.result) {
            return Err(replay_contract_mismatch(
                "external adoption read assertion evidence failed",
            ));
        }
    }
    for (record, assertion) in evidence
        .event_assertion_evidence
        .iter()
        .zip(policy.initial_event_assertions.iter())
    {
        if assertion.from_block.is_some() || assertion.to_block.is_some() {
            return Err(replay_contract_mismatch(
                "external adoption event assertion block bounds are not retained in evidence",
            ));
        }
        verify_external_source_evidence(context, &record.source)?;
        if record.result.event.as_str() != assertion.event.as_str()
            || record.result.min_count != assertion.min_count
        {
            return Err(replay_contract_mismatch(
                "external adoption event assertion evidence does not match certified policy",
            ));
        }
        require_validation_event_result_canonical_passed(&record.result)
            .map_err(replay_adapter_error)?;
        if !validation_event_result_passes(&record.result) {
            return Err(replay_contract_mismatch(
                "external adoption event assertion evidence failed",
            ));
        }
    }
    if required_stage == ContractLifecycleStage::Configured
        && evidence.read_assertion_evidence.is_empty()
        && evidence.event_assertion_evidence.is_empty()
        && !policy.allow_external_claimed_configured
    {
        return Err(replay_contract_mismatch(
            "configured external adoption lacks required assertion authority",
        ));
    }
    Ok(())
}

pub(super) fn verify_validation_results_match_action(
    output: &ContextBoundValidationReport,
    action: &ValidateAction,
) -> replay::Result<()> {
    if output.read_results.len() != action.read_assertions().len()
        || output.event_results.len() != action.event_assertions().len()
    {
        return Err(replay_contract_mismatch(
            "validation report result count does not match certified validate action",
        ));
    }
    for (result, assertion) in output.read_results.iter().zip(action.read_assertions()) {
        if result.function != assertion.function.as_str()
            || result.args != assertion.args
            || result.expected != assertion.expected
        {
            return Err(replay_contract_mismatch(
                "validation read result does not match certified validate action",
            ));
        }
    }
    for (result, assertion) in output.event_results.iter().zip(action.event_assertions()) {
        if result.event != assertion.event.as_str() || result.min_count != assertion.min_count {
            return Err(replay_contract_mismatch(
                "validation event result does not match certified validate action",
            ));
        }
    }
    Ok(())
}

pub(super) fn verify_validation_source_evidence(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    read_results: &[ValidationReadResult],
    read_evidence: &[ExternalReadAssertionEvidence],
    event_results: &[ValidationEventResult],
    event_evidence: &[ExternalEventAssertionEvidence],
) -> replay::Result<()> {
    if read_results.len() != read_evidence.len() || event_results.len() != event_evidence.len() {
        return Err(replay_contract_mismatch(
            "validation report evidence count does not match assertion results",
        ));
    }
    for (result, evidence) in read_results.iter().zip(read_evidence) {
        verify_external_source_evidence(context, &evidence.source)?;
        if result != &evidence.result {
            return Err(replay_contract_mismatch(
                "validation read evidence does not match report result",
            ));
        }
    }
    for (result, evidence) in event_results.iter().zip(event_evidence) {
        verify_external_source_evidence(context, &evidence.source)?;
        if result != &evidence.result {
            return Err(replay_contract_mismatch(
                "validation event evidence does not match report result",
            ));
        }
    }
    Ok(())
}

fn verify_external_source_evidence(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    evidence: &ExternalEvmSourceEvidence,
) -> replay::Result<()> {
    if evidence.network_id != context.value().network.network_id.to_string()
        || evidence.expected_chain_id != context.value().network.expected_chain_id()
        || evidence.observed_chain_id != context.value().network.expected_chain_id()
    {
        return Err(replay_contract_mismatch(
            "external EVM source evidence does not match certified context",
        ));
    }
    EvmNetworkId::new(evidence.network_id.as_str()).map_err(replay_adapter_error)?;
    Ok(())
}
