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
    let (configuration_read_results, configuration_event_results) = input
        .configured
        .asserted_configuration_snapshot
        .as_ref()
        .map(|snapshot| {
            (
                snapshot.read_results.clone(),
                snapshot.event_results.clone(),
            )
        })
        .unwrap_or_default();

    Ok(ContractValidationReadResponse {
        response_version: 1,
        context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
        configured_input_digest: request.configured_input_digest.clone(),
        evm_network_context_ref: evm_network_context_ref(&context.value().network)?,
        resource_stage: ContractLifecycleStage::Configured,
        observed_chain_id: chain.chain_id,
        client_version: chain.client_version.unwrap_or_else(|| "unknown".to_owned()),
        configuration_read_results,
        configuration_event_results,
        read_results: evaluated.read_results,
        event_results: evaluated.event_results,
        validation_read_evidence: evaluated.read_evidence,
        validation_event_evidence: evaluated.event_evidence,
        evidence_refs: input.configured.configure_or_import_evidence.clone(),
    })
}

pub(super) async fn import_deployed_with_reads(
    reads: EvmContractReadProviders<'_>,
    import: &ImportDeployedSpec,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    source_run_registry: Option<&mfm_certify::CertificationRegistry>,
) -> Result<DeployedContractInstance> {
    match import {
        ImportDeployedSpec::FromMfmRun { source, evidence } => {
            let imported = import_source_run_value::<DeployedContractInstance>(
                artifacts,
                source,
                evidence,
                ContractLifecycleStage::Deployed,
                source_run_registry,
            )
            .await?;
            ImportDeployedContractState::admit_verified_mfm_run_import(import, imported, context)
                .map_err(Into::into)
        }
        ImportDeployedSpec::AdoptExternalAddress { adoption } => {
            let verified = verify_external_adoption(
                reads,
                adoption,
                context,
                ContractLifecycleStage::Deployed,
            )
            .await?;
            ImportDeployedContractState::admit_verified_external_adoption(
                import,
                context,
                verified.evidence,
                None,
            )
            .map_err(Into::into)
        }
    }
}

pub(super) async fn import_configured_with_reads(
    reads: EvmContractReadProviders<'_>,
    import: &ImportConfiguredSpec,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    artifact: Option<&ContractArtifactConfig>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    source_run_registry: Option<&mfm_certify::CertificationRegistry>,
) -> Result<ConfiguredContractInstance> {
    match import {
        ImportConfiguredSpec::FromMfmRun { source, evidence } => {
            let imported = import_source_run_value::<ConfiguredContractInstance>(
                artifacts,
                source,
                evidence,
                ContractLifecycleStage::Configured,
                source_run_registry,
            )
            .await?;
            ImportConfiguredContractState::admit_verified_mfm_run_import(import, imported, context)
                .map_err(Into::into)
        }
        ImportConfiguredSpec::AdoptExternalAddress { adoption } => {
            let mut verified = verify_external_adoption(
                reads,
                adoption,
                context,
                ContractLifecycleStage::Configured,
            )
            .await?;
            let snapshot = if validation_assertions_required(
                &adoption.evidence_policy.initial_read_assertions,
                &adoption.evidence_policy.initial_event_assertions,
            ) {
                let assertion_context = prepare_validation_assertion_context(
                    artifact,
                    adoption.address.as_str(),
                    true,
                )?;
                let evaluated = evaluate_assertions(
                    reads,
                    assertion_context.as_ref(),
                    &adoption.evidence_policy.initial_read_assertions,
                    &adoption.evidence_policy.initial_event_assertions,
                )
                .await?;
                let all_passed = evaluated.read_results.iter().all(|result| result.passed)
                    && evaluated.event_results.iter().all(|result| result.passed);
                if !all_passed {
                    return Err(EvmContractAdapterError::ExternalAdoptionAssertionsFailed);
                }
                verified.evidence.read_assertion_evidence = evaluated.read_evidence;
                verified.evidence.event_assertion_evidence = evaluated.event_evidence;
                Some(ConfigurationSnapshot {
                    read_results: evaluated.read_results,
                    event_results: evaluated.event_results,
                })
            } else if adoption.evidence_policy.allow_external_claimed_configured {
                None
            } else {
                return Err(EvmContractAdapterError::ExternalConfiguredClaimNotAllowed);
            };
            ImportConfiguredContractState::admit_verified_external_adoption(
                import,
                context,
                verified.evidence,
                snapshot,
                None,
            )
            .map_err(Into::into)
        }
    }
}

struct VerifiedExternalAdoption {
    evidence: ExternalAdoptionEvidence,
}

async fn verify_external_adoption(
    reads: EvmContractReadProviders<'_>,
    adoption: &mfm_evm_contract_model::AdoptExternalAddress,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    resource_stage: ContractLifecycleStage,
) -> Result<VerifiedExternalAdoption> {
    let chain = read_chain_identity(reads.chain_identity).await?;
    let mut code_read_evidence = None;
    if adoption.evidence_policy.require_code
        || adoption.evidence_policy.expected_code_hash.is_some()
    {
        let address = parse_address(adoption.address.as_str(), "address")
            .map_err(|error| EvmContractAdapterError::Model(error.message))?;
        let block =
            adoption
                .evidence_policy
                .block_anchor
                .clone()
                .unwrap_or(ModelBlockSelector::Tag {
                    tag: BlockTag::Latest,
                });
        let response = reads
            .code
            .read_code(&EvmCodeReadRequest::new(
                address,
                block_selector(Some(&block), true)?,
            ))
            .await?;
        if adoption.evidence_policy.require_code && response.code.is_empty() {
            return Err(EvmContractAdapterError::ExternalCodeMissing);
        }
        let observed = EvmCodeHash::new(format!("{:?}", response.code_hash))
            .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
        if let Some(expected) = &adoption.evidence_policy.expected_code_hash {
            if &observed != expected {
                return Err(EvmContractAdapterError::ExternalCodeHashMismatch);
            }
        }
        code_read_evidence = Some(ExternalCodeReadEvidence {
            address: adoption.address.clone(),
            block,
            source: external_evm_source_evidence(&response.evidence),
            observed_code_hash: observed,
            observed_code_byte_len: response.code.len() as u64,
        });
    }
    let evidence = ExternalAdoptionEvidence {
        evidence_policy_digest: digest_for_value(&adoption.evidence_policy)?,
        context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
        evm_network_context_ref: evm_network_context_ref(&context.value().network)?,
        resource_stage,
        observed_chain_id: chain.chain_id,
        code_read_evidence,
        read_assertion_evidence: Vec::new(),
        event_assertion_evidence: Vec::new(),
    };
    Ok(VerifiedExternalAdoption { evidence })
}

fn external_evm_source_evidence(evidence: &RedactedEvmSourceEvidence) -> ExternalEvmSourceEvidence {
    ExternalEvmSourceEvidence {
        network_id: evidence.network_id.to_string(),
        expected_chain_id: evidence.expected_chain_id,
        observed_chain_id: evidence.observed_chain_id,
        source_ref: evidence.source_ref.to_string(),
        policy_id: evidence.policy_id.to_string(),
    }
}

pub(super) fn import_configured_requires_artifact(import: &ImportConfiguredSpec) -> bool {
    match import {
        ImportConfiguredSpec::FromMfmRun { .. } => false,
        ImportConfiguredSpec::AdoptExternalAddress { adoption } => validation_assertions_required(
            &adoption.evidence_policy.initial_read_assertions,
            &adoption.evidence_policy.initial_event_assertions,
        ),
    }
}

async fn import_source_run_value<T>(
    artifacts: &dyn store::RetainedArtifactReadProvider,
    source: &ImportFromMfmRun,
    evidence: &ImportFromMfmRunEvidence,
    required_stage: ContractLifecycleStage,
    source_run_registry: Option<&mfm_certify::CertificationRegistry>,
) -> Result<T>
where
    T: ContextBoundOutput + DeserializeOwned,
{
    let source_run_registry = source_run_registry.ok_or_else(|| {
        EvmContractAdapterError::SourceRunImportEvidence(
            "source-run import requires trusted certification registry authority".to_owned(),
        )
    })?;
    let source_spec =
        read_lifecycle_evidence_artifact(artifacts, &evidence.source_spec_artifact_ref).await?;
    let certificate =
        read_lifecycle_evidence_artifact(artifacts, &evidence.source_spec_certificate_ref).await?;
    let stream =
        read_lifecycle_evidence_artifact(artifacts, &evidence.source_run_stream_ref).await?;
    let source_value = read_lifecycle_evidence_artifact(
        artifacts,
        &evidence.source_value_artifact_ref_or_inline_canonical_value,
    )
    .await?;
    let value = serde_json::from_slice::<T>(source_value.bytes())
        .map_err(|error| EvmContractAdapterError::SourceRunImportEvidence(error.to_string()))?;
    if value.context_ref() != evidence.source_context_ref.as_context_ref()
        || value.context_resource_kind()
            != mfm_evm_contract_model::contract_instance_resource_kind()
        || value.context_stage() != context_stage_for_lifecycle_stage(required_stage)
        || digest_for_value(&value)? != evidence.source_value_digest
    {
        return Err(EvmContractAdapterError::ContextMismatch);
    }
    let committed = decode_source_run_committed_stream(
        source,
        stream.bytes(),
        source_value.evidence(),
        source_value.bytes(),
    )?;
    validate_source_run_authority(SourceRunAuthorityEvidence {
        source,
        evidence,
        source_spec_bytes: source_spec.bytes(),
        certificate_bytes: certificate.bytes(),
        committed: &committed,
        source_value_artifact: source_value.evidence(),
        required_stage,
        source_run_registry,
    })?;
    Ok(value)
}

pub(super) struct SourceRunAuthorityEvidence<'a> {
    pub(super) source: &'a ImportFromMfmRun,
    pub(super) evidence: &'a ImportFromMfmRunEvidence,
    pub(super) source_spec_bytes: &'a [u8],
    pub(super) certificate_bytes: &'a [u8],
    pub(super) committed: &'a store::CommittedRunStream,
    pub(super) source_value_artifact: &'a store::ArtifactEvidenceRef,
    pub(super) required_stage: ContractLifecycleStage,
    pub(super) source_run_registry: &'a mfm_certify::CertificationRegistry,
}

pub(super) fn validate_source_run_authority(
    authority: SourceRunAuthorityEvidence<'_>,
) -> Result<()> {
    let SourceRunAuthorityEvidence {
        source,
        evidence,
        source_spec_bytes,
        certificate_bytes,
        committed,
        source_value_artifact,
        required_stage,
        source_run_registry,
    } = authority;
    let certified_source_spec =
        mfm_certify::verify_persisted_spec_certificate_with_trusted_registry(
            source_spec_bytes,
            certificate_bytes,
            source_run_registry,
        )
        .map_err(|error| EvmContractAdapterError::SourceRunImportEvidence(error.to_string()))?;
    let run_admitted = source_run_admitted(committed)?;
    let source_run_id = source
        .source_run_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?;
    let source_spec_hash = source
        .source_spec_hash
        .typed()
        .map_err(EvmContractAdapterError::Model)?;

    if committed.run_id() != &source_run_id
        || run_admitted.run_id != source_run_id
        || run_admitted.spec_hash != source_spec_hash
        || certified_source_spec.spec_hash() != &source_spec_hash
        || !lifecycle_ref_matches_run_artifact(
            &evidence.source_spec_artifact_ref,
            &run_admitted.spec_artifact,
        )?
        || !lifecycle_ref_matches_run_artifact(
            &evidence.source_spec_certificate_ref,
            &run_admitted.certificate_artifact,
        )?
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run committed stream does not match trusted source spec authority".to_owned(),
        ));
    }

    let terminal = source_run_terminal_event_from_committed_stream(
        committed,
        evidence,
        source_value_artifact,
        &certified_source_spec,
        required_stage,
    )?;

    verify_source_terminal_event_against_certified_spec(
        &certified_source_spec,
        &terminal,
        required_stage,
    )?;

    Ok(())
}

pub(super) fn decode_source_run_committed_stream(
    source: &ImportFromMfmRun,
    stream_bytes: &[u8],
    source_value_artifact: &store::ArtifactEvidenceRef,
    source_value_bytes: &[u8],
) -> Result<store::CommittedRunStream> {
    let source_run_id = source
        .source_run_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?;
    let mut artifact_bytes = store::ArtifactByteAuthorityMap::new();
    artifact_bytes.insert(
        (
            source_value_artifact.artifact_id.clone(),
            source_value_artifact.evidence_hash().map_err(|error| {
                EvmContractAdapterError::SourceRunImportEvidence(error.to_string())
            })?,
        ),
        (source_value_bytes.to_vec(), source_value_artifact.clone()),
    );
    store::committed_run_stream_from_canonical_json_slice(
        &source_run_id,
        stream_bytes,
        &artifact_bytes,
    )
    .map_err(|error| EvmContractAdapterError::SourceRunImportEvidence(error.to_string()))
}

pub(super) fn source_run_admitted(
    committed: &store::CommittedRunStream,
) -> Result<&events::RunAdmitted> {
    let mut admitted = committed.events().iter().filter_map(|event| {
        if let events::KernelEventPayload::RunAdmitted(payload) = event.payload() {
            Some(payload.as_ref())
        } else {
            None
        }
    });
    let Some(first) = admitted.next() else {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run committed stream is missing RunAdmitted".to_owned(),
        ));
    };
    if admitted.next().is_some() {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run committed stream contains multiple RunAdmitted events".to_owned(),
        ));
    }
    Ok(first)
}

fn lifecycle_ref_matches_run_artifact(
    evidence: &LifecycleArtifactEvidenceRef,
    artifact: &events::RunArtifactEvidenceRef,
) -> Result<bool> {
    Ok(evidence
        .artifact_id()
        .map_err(EvmContractAdapterError::Model)?
        == artifact.artifact_id
        && evidence
            .content_digest()
            .map_err(EvmContractAdapterError::Model)?
            == artifact.content_digest
        && evidence.byte_len() == artifact.byte_len
        && evidence
            .schema_id()
            .map_err(EvmContractAdapterError::Model)?
            == artifact.schema_id
        && evidence
            .semantic_type_id()
            .map_err(EvmContractAdapterError::Model)?
            == artifact.semantic_type_id)
}

fn source_run_terminal_event_from_committed_stream(
    committed: &store::CommittedRunStream,
    evidence: &ImportFromMfmRunEvidence,
    source_value_artifact: &store::ArtifactEvidenceRef,
    certified: &mfm_certify::CertifiedTypedSpec,
    required_stage: ContractLifecycleStage,
) -> Result<SourceRunTerminalEvent> {
    let event_id = evidence
        .source_terminal_cell_or_output_event_ref
        .typed()
        .map_err(EvmContractAdapterError::Model)?;
    let event = committed
        .events()
        .iter()
        .find(|event| event.event_id() == &event_id)
        .ok_or_else(|| {
            EvmContractAdapterError::SourceRunImportEvidence(
                "source-run committed stream does not contain claimed terminal event".to_owned(),
            )
        })?;

    let terminal = match (event.payload(), &evidence.source_cell_or_output_id) {
        (
            events::KernelEventPayload::CellProduced(payload),
            SourceCellOrOutputRef::Cell { cell_id: _ },
        ) => source_run_terminal_event_from_cell(
            event.event_id(),
            payload,
            evidence,
            source_value_artifact,
            required_stage,
        )?,
        (
            events::KernelEventPayload::PublicOutputProduced(payload),
            SourceCellOrOutputRef::PublicOutput { output_key },
        ) => source_run_terminal_event_from_public_output(
            event.event_id(),
            payload,
            output_key,
            evidence,
            source_value_artifact,
            certified,
            required_stage,
        )?,
        _ => {
            return Err(EvmContractAdapterError::SourceRunImportEvidence(
                "source-run committed stream terminal event kind does not match import evidence"
                    .to_owned(),
            ));
        }
    };
    if source_run_terminal_event_matches(&terminal, evidence, required_stage) {
        Ok(terminal)
    } else {
        Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run committed stream terminal event does not match import evidence".to_owned(),
        ))
    }
}

fn source_run_terminal_event_from_cell(
    event_id: &EventId,
    payload: &events::CellProduced,
    evidence: &ImportFromMfmRunEvidence,
    source_value_artifact: &store::ArtifactEvidenceRef,
    required_stage: ContractLifecycleStage,
) -> Result<SourceRunTerminalEvent> {
    if payload.artifact_id != source_value_artifact.artifact_id
        || payload.content_digest != source_value_artifact.digest
        || Some(&payload.schema_id) != source_value_artifact.schema_id.as_ref()
        || Some(&payload.semantic_type_id) != source_value_artifact.semantic_type_id.as_ref()
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal cell artifact does not match retained source value".to_owned(),
        ));
    }
    let spec::CellContextSpec::Bound { context_ref, .. } = &payload.context else {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal cell is not context-bound".to_owned(),
        ));
    };
    Ok(SourceRunTerminalEvent {
        source_terminal_cell_or_output_event_ref: mfm_evm_contract_model::LifecycleEventIdRef::from(
            event_id.clone(),
        ),
        source_cell_or_output_id: SourceCellOrOutputRef::Cell {
            cell_id: mfm_evm_contract_model::LifecycleCellIdRef::from(payload.cell_id.clone()),
        },
        source_cell_schema_id: mfm_evm_contract_model::ArtifactEvidenceSchemaId::from(
            payload.schema_id.clone(),
        ),
        source_cell_semantic_type_id: mfm_evm_contract_model::ArtifactEvidenceSemanticTypeId::from(
            payload.semantic_type_id.clone(),
        ),
        source_producer_descriptor_id: evidence.source_producer_descriptor_id.clone(),
        source_stage: required_stage,
        source_context_ref: mfm_values::ContextRefValue::from(context_ref.clone()),
        source_context_descriptor_id: evidence.source_context_descriptor_id.clone(),
        source_value_digest: ContractProfileDigestRef::from(payload.content_digest.clone()),
    })
}

fn source_run_terminal_event_from_public_output(
    event_id: &EventId,
    payload: &events::PublicOutputProduced,
    output_key: &mfm_evm_contract_model::ArtifactPort,
    evidence: &ImportFromMfmRunEvidence,
    source_value_artifact: &store::ArtifactEvidenceRef,
    certified: &mfm_certify::CertifiedTypedSpec,
    required_stage: ContractLifecycleStage,
) -> Result<SourceRunTerminalEvent> {
    let cell = payload
        .cells
        .iter()
        .find(|cell| cell.public_field_path.as_str() == output_key.as_str())
        .ok_or_else(|| {
            EvmContractAdapterError::SourceRunImportEvidence(
                "source-run public output event does not contain claimed output key".to_owned(),
            )
        })?;
    if cell.artifact_id != source_value_artifact.artifact_id
        || cell.content_digest != source_value_artifact.digest
        || Some(&cell.schema_id) != source_value_artifact.schema_id.as_ref()
        || Some(&cell.semantic_type_id) != source_value_artifact.semantic_type_id.as_ref()
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run public output artifact does not match retained source value".to_owned(),
        ));
    }
    let certified_cell = certified
        .validated_spec()
        .graph()
        .cell(&cell.cell_id)
        .ok_or_else(|| {
            EvmContractAdapterError::SourceRunImportEvidence(
                "source-run public output cell is not declared by source spec".to_owned(),
            )
        })?;
    let spec::CellContextSpec::Bound { context_ref, .. } = &certified_cell.context else {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run public output cell is not context-bound".to_owned(),
        ));
    };
    Ok(SourceRunTerminalEvent {
        source_terminal_cell_or_output_event_ref: mfm_evm_contract_model::LifecycleEventIdRef::from(
            event_id.clone(),
        ),
        source_cell_or_output_id: SourceCellOrOutputRef::PublicOutput {
            output_key: output_key.clone(),
        },
        source_cell_schema_id: mfm_evm_contract_model::ArtifactEvidenceSchemaId::from(
            cell.schema_id.clone(),
        ),
        source_cell_semantic_type_id: mfm_evm_contract_model::ArtifactEvidenceSemanticTypeId::from(
            cell.semantic_type_id.clone(),
        ),
        source_producer_descriptor_id: evidence.source_producer_descriptor_id.clone(),
        source_stage: required_stage,
        source_context_ref: mfm_values::ContextRefValue::from(context_ref.clone()),
        source_context_descriptor_id: evidence.source_context_descriptor_id.clone(),
        source_value_digest: ContractProfileDigestRef::from(cell.content_digest.clone()),
    })
}

fn verify_source_terminal_event_against_certified_spec(
    certified: &mfm_certify::CertifiedTypedSpec,
    terminal: &SourceRunTerminalEvent,
    required_stage: ContractLifecycleStage,
) -> Result<()> {
    let cell_id = match &terminal.source_cell_or_output_id {
        SourceCellOrOutputRef::Cell { cell_id } => {
            cell_id.typed().map_err(EvmContractAdapterError::Model)?
        }
        SourceCellOrOutputRef::PublicOutput { output_key } => certified
            .envelope()
            .spec
            .public_outputs
            .outputs
            .iter()
            .find(|output| output.public_field_path.as_str() == output_key.as_str())
            .map(|output| output.cell_id.clone())
            .ok_or_else(|| {
                EvmContractAdapterError::SourceRunImportEvidence(
                    "source-run public output is not declared by the certified source spec"
                        .to_owned(),
                )
            })?,
    };
    let graph = certified.validated_spec().graph();
    let cell = graph.cell(&cell_id).ok_or_else(|| {
        EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal cell is not declared by the certified source spec".to_owned(),
        )
    })?;

    if terminal
        .source_cell_schema_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?
        != cell.schema_id
        || terminal
            .source_cell_semantic_type_id
            .typed()
            .map_err(EvmContractAdapterError::Model)?
            != cell.semantic_type_id
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal type evidence does not match certified source cell".to_owned(),
        ));
    }

    let spec::CellContextSpec::Bound {
        context_ref,
        resource_kind,
        stage,
        producer,
    } = &cell.context
    else {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal cell is not context-bound".to_owned(),
        ));
    };
    if context_ref != terminal.source_context_ref.as_context_ref()
        || resource_kind != mfm_evm_contract_model::contract_instance_resource_kind()
        || stage != context_stage_for_lifecycle_stage(required_stage)
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal context does not match certified source cell".to_owned(),
        ));
    }
    let source_context_descriptor_id = terminal
        .source_context_descriptor_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?;
    if !certified.envelope().spec.contexts.iter().any(|context| {
        &context.context_ref == context_ref
            && context.context_descriptor_id == source_context_descriptor_id
    }) {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal context descriptor is not certified by source spec".to_owned(),
        ));
    }

    let spec::CellProducer::Node(node_id) = &cell.producer else {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal cell was not produced by a certified lifecycle node".to_owned(),
        ));
    };
    let node = graph.forward_node(node_id).ok_or_else(|| {
        EvmContractAdapterError::SourceRunImportEvidence(
            "source-run producer node is not certified by source spec".to_owned(),
        )
    })?;
    let source_producer_descriptor_id = terminal
        .source_producer_descriptor_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?;
    if node.output_cell != cell.cell_id
        || !producer
            .producer_descriptor_ids
            .iter()
            .any(|descriptor_id| descriptor_id == &source_producer_descriptor_id)
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run producer descriptor is not certified for the terminal cell".to_owned(),
        ));
    }
    if node.context
        != (spec::NodeContextSpec::Required {
            context_ref: context_ref.clone(),
        })
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run producer node context does not match terminal cell context".to_owned(),
        ));
    }
    Ok(())
}

fn source_run_terminal_event_matches(
    event: &SourceRunTerminalEvent,
    evidence: &ImportFromMfmRunEvidence,
    required_stage: ContractLifecycleStage,
) -> bool {
    event.source_terminal_cell_or_output_event_ref
        == evidence.source_terminal_cell_or_output_event_ref
        && event.source_cell_or_output_id == evidence.source_cell_or_output_id
        && event.source_cell_schema_id == evidence.source_cell_schema_id
        && event.source_cell_semantic_type_id == evidence.source_cell_semantic_type_id
        && event.source_producer_descriptor_id == evidence.source_producer_descriptor_id
        && event.source_stage == required_stage
        && event.source_stage == evidence.source_stage
        && event.source_context_ref == evidence.source_context_ref
        && event.source_context_descriptor_id == evidence.source_context_descriptor_id
        && event.source_value_digest == evidence.source_value_digest
}

async fn read_lifecycle_evidence_artifact(
    artifacts: &dyn store::RetainedArtifactReadProvider,
    evidence: &LifecycleArtifactEvidenceRef,
) -> Result<store::VerifiedRunArtifactBytes> {
    let requirement = lifecycle_artifact_requirement(evidence)?;
    artifacts
        .read_retained_artifact(&requirement)
        .await
        .map_err(|error| EvmContractAdapterError::SourceRunImportEvidence(error.to_string()))
}

pub(super) fn lifecycle_artifact_requirement(
    evidence: &LifecycleArtifactEvidenceRef,
) -> Result<events::EventArtifactRequirement> {
    let artifact_id = evidence
        .artifact_id()
        .map_err(EvmContractAdapterError::Model)?;
    let digest = evidence
        .content_digest()
        .map_err(EvmContractAdapterError::Model)?;
    let evidence_hash = evidence
        .evidence_hash()
        .map_err(EvmContractAdapterError::Model)?;
    let schema_id = evidence
        .schema_id()
        .map_err(EvmContractAdapterError::Model)?;
    let semantic_type_id = evidence
        .semantic_type_id()
        .map_err(EvmContractAdapterError::Model)?;
    Ok(events::EventArtifactRequirement {
        source: events::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id,
        evidence_hash,
        digest: Some(digest),
        byte_len: Some(evidence.byte_len()),
        media_type: None,
        schema_id,
        semantic_type_id,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: None,
    })
}

pub(super) fn run_artifact_requirement(
    source: events::EventArtifactReferenceSource,
    artifact: &events::RunArtifactEvidenceRef,
) -> events::EventArtifactRequirement {
    events::EventArtifactRequirement {
        source,
        artifact_id: artifact.artifact_id.clone(),
        evidence_hash: artifact.evidence_hash.clone(),
        digest: Some(artifact.content_digest.clone()),
        byte_len: Some(artifact.byte_len),
        media_type: Some(artifact.media_type.clone()),
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(artifact.role),
    }
}

pub(super) fn context_stage_for_lifecycle_stage(
    stage: ContractLifecycleStage,
) -> &'static mfm_ids::ContextStage {
    match stage {
        ContractLifecycleStage::Deployed => deployed_contract_stage(),
        ContractLifecycleStage::Configured => configured_contract_stage(),
    }
}

pub(super) fn digest_for_value<T>(value: &T) -> Result<ContractProfileDigestRef>
where
    T: Serialize,
{
    let json = serde_json::to_string(value)
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| ContractProfileDigestRef::from(canonical.content_digest()))
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))
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
    read_evidence: Vec<ExternalReadAssertionEvidence>,
    event_evidence: Vec<ExternalEventAssertionEvidence>,
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
        read_evidence.push(ExternalReadAssertionEvidence {
            source: external_evm_source_evidence(&response.evidence),
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
        event_evidence.push(ExternalEventAssertionEvidence {
            source: external_evm_source_evidence(&logs.evidence),
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
