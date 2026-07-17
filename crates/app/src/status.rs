use super::*;

pub(super) fn run_status_from_projection(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projection: &store::ProjectionSnapshot,
) -> Result<RunResponse, PublicError> {
    let spec_hash = run_admitted_spec_hash(stream)?;
    run_response_from_projection_with_spec_hash(
        run_id,
        runtime_spec,
        stream,
        projection,
        &spec_hash,
        "observed",
    )
}

fn run_response_from_projection_with_spec_hash(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projection: &store::ProjectionSnapshot,
    spec_hash: &SpecHash,
    scheduler_status: &str,
) -> Result<RunResponse, PublicError> {
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    let saga =
        projection.derive_saga_projection(run_id, &runtime_spec.spec().saga, &terminal_policies)?;
    Ok(RunResponse {
        run_id: run_id.as_str().to_owned(),
        spec_hash: spec_hash.as_str().to_owned(),
        run_mode: run_mode_status(saga.run_mode),
        saga: saga_status_with_resources(runtime_spec.spec(), projection, &saga),
        attempt_dispositions: attempt_dispositions(projection),
        scheduler_status: scheduler_status.to_owned(),
        head_seq: stream_head(stream),
    })
}

pub(super) fn run_stream_response_from_verified_context(
    context: &VerifiedRunReadContext,
) -> RunStreamResponse {
    let events = context.events();
    RunStreamResponse {
        run_id: context.view().run_id().as_str().to_owned(),
        head_seq: stream_head(events),
        events: events.iter().map(run_event_ref).collect(),
    }
}

/// Builds typed public-output read authority from certified runtime authority, verified run-history
/// view, rebuilt projection, and verified typed artifact evidence.
pub async fn public_output_read_authority_for_run(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    runtime_spec: &CertifiedRuntimeSpec,
    verified_view: &VerifiedRunHistoryView,
    public_schema_id: &SchemaId,
) -> Result<PublicOutputReadAuthority, PublicError> {
    if runtime_spec.spec_hash() != verified_view.spec_hash() {
        return Err(PublicError::new(
            ErrorClass::Internal,
            "PublicOutputAuthorityMismatch",
            "verified history spec hash does not match certified runtime authority",
        ));
    }
    let projection = verified_view.projection_snapshot();
    let public_output = projection
        .public_output(verified_view.run_id(), public_schema_id)
        .ok_or_else(|| {
            PublicError::not_found(
                "PublicOutputNotFound",
                "typed public output was not found for the requested schema",
            )
        })?;
    let store::PublicOutputProjection::Produced {
        event_id,
        rendered_digest,
        rendered_artifact_id,
    } = public_output
    else {
        return Err(PublicError::new(
            ErrorClass::Conflict,
            "PublicOutputRenderFailed",
            "typed public output render failed",
        ));
    };

    let payload =
        public_output_payload_from_stream(verified_view.events(), event_id, public_schema_id)?;
    if &payload.spec_hash != runtime_spec.spec_hash()
        || &payload.public_schema_id != public_schema_id
        || public_schema_id != &runtime_spec.spec().public_outputs.public_schema_id
    {
        return Err(PublicError::new(
            ErrorClass::Internal,
            "PublicOutputAuthorityMismatch",
            "typed public-output evidence does not match certified runtime authority",
        ));
    }
    verify_public_output_authority_artifacts(
        artifacts,
        payload,
        rendered_artifact_id.as_ref(),
        rendered_digest,
    )
    .await?;

    Ok(PublicOutputReadAuthority {
        run_id: verified_view.run_id().clone(),
        public_schema_id: public_schema_id.clone(),
        event_id: event_id.clone(),
        rendered_digest: rendered_digest.clone(),
        rendered_artifact_id: rendered_artifact_id.clone(),
        payload: payload.clone(),
    })
}

pub(super) async fn verify_public_output_authority_artifacts(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    payload: &events::PublicOutputProduced,
    rendered_artifact_id: Option<&ArtifactId>,
    rendered_digest: &ContentDigest,
) -> Result<(), PublicError> {
    for cell in &payload.cells {
        let artifact = artifacts
            .read_retained_artifact(&public_output_cell_artifact_requirement(cell))
            .await?;
        let evidence = artifact.evidence().clone();
        verify_public_output_cell_evidence(cell, &evidence)?;
    }
    if let Some(artifact_id) = rendered_artifact_id {
        let json_media_type = public_output_json_media_type()?;
        let requirement = public_output_rendered_artifact_requirement(
            payload,
            artifact_id,
            rendered_digest,
            json_media_type,
        )?;
        let artifact = artifacts.read_retained_artifact(&requirement).await?;
        let evidence = artifact.evidence().clone();
        verify_public_output_rendered_artifact_evidence(
            &evidence,
            artifact_id,
            rendered_digest,
            payload,
        )?;
    }
    Ok(())
}

/// Renders typed public output from app-verified read authority and typed artifact bytes.
pub async fn render_public_output(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    authority: &PublicOutputReadAuthority,
) -> Result<PublicOutputResponse, PublicError> {
    let json = match authority.rendered_artifact_id() {
        Some(artifact_id) => Some(
            load_public_output_json(
                artifacts,
                artifact_id,
                authority.rendered_digest(),
                &authority.payload,
            )
            .await?,
        ),
        None => Some(render_public_output_json_from_authority(artifacts, authority).await?),
    };
    Ok(PublicOutputResponse {
        run_id: authority.run_id().as_str().to_owned(),
        public_schema_id: authority.public_schema_id().as_str().to_owned(),
        event_id: authority.event_id().as_str().to_owned(),
        rendered_digest: authority.rendered_digest().as_str().to_owned(),
        rendered_artifact_id: authority
            .rendered_artifact_id()
            .map(|artifact_id| artifact_id.as_str().to_owned()),
        json,
    })
}

pub(super) fn public_output_payload_from_stream<'a>(
    stream: &'a [store::KernelEventEnvelope],
    event_id: &EventId,
    public_schema_id: &SchemaId,
) -> Result<&'a events::PublicOutputProduced, PublicError> {
    stream
        .iter()
        .find_map(|event| {
            if event.event_id() != event_id {
                return None;
            }
            match event.payload() {
                events::KernelEventPayload::PublicOutputProduced(payload)
                    if &payload.public_schema_id == public_schema_id =>
                {
                    Some(payload)
                }
                _ => None,
            }
        })
        .ok_or_else(|| {
            PublicError::new(
                ErrorClass::Internal,
                "PublicOutputProjectionMismatch",
                "typed public-output projection does not match the authoritative run stream",
            )
        })
}

pub(super) async fn render_public_output_json_from_authority(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    authority: &PublicOutputReadAuthority,
) -> Result<Value, PublicError> {
    let mut root = Map::new();
    for cell in &authority.payload.cells {
        let artifact = artifacts
            .read_retained_artifact(&public_output_cell_artifact_requirement(cell))
            .await?;
        let evidence = artifact.evidence().clone();
        verify_public_output_cell_evidence(cell, &evidence)?;
        let bytes = artifact.into_bytes();
        let value = serde_json::from_slice(&bytes).map_err(|error| {
            let _ = error;
            PublicError::backend(
                ErrorClass::Internal,
                "PublicOutputDecodeFailed",
                "Typed public-output cell artifact was not JSON",
            )
        })?;
        insert_public_output_value(&mut root, cell.public_field_path.as_str(), value)?;
    }
    Ok(Value::Object(root))
}

pub(super) fn verify_public_output_cell_evidence(
    cell: &events::NamedTypedCellRef,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<(), PublicError> {
    validate_artifact_requirement_for_app(
        public_output_cell_artifact_requirement(cell),
        evidence,
        ErrorClass::Internal,
        "PublicOutputArtifactMismatch",
        "typed public-output cell artifact evidence does not match the event cell reference",
    )
}

pub(super) fn insert_public_output_value(
    root: &mut Map<String, Value>,
    path: &str,
    value: Value,
) -> Result<(), PublicError> {
    let mut parts = path.split('.').peekable();
    let mut current = root;
    let mut value = Some(value);

    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            match current.entry(part.to_owned()) {
                Entry::Vacant(entry) => {
                    entry.insert(value.take().expect("public output value inserted once"));
                    return Ok(());
                }
                Entry::Occupied(_) => {
                    return Err(public_output_artifact_mismatch(
                        "typed public-output field paths collide",
                    ));
                }
            }
        }

        let entry = current
            .entry(part.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        let Value::Object(next) = entry else {
            return Err(public_output_artifact_mismatch(
                "typed public-output field path collides with a scalar value",
            ));
        };
        current = next;
    }

    Err(public_output_artifact_mismatch(
        "typed public-output field path was empty",
    ))
}

pub(super) async fn load_public_output_json(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    artifact_id: &ArtifactId,
    rendered_digest: &mfm_ids::ContentDigest,
    payload: &events::PublicOutputProduced,
) -> Result<serde_json::Value, PublicError> {
    let json_media_type = public_output_json_media_type()?;
    let requirement = public_output_rendered_artifact_requirement(
        payload,
        artifact_id,
        rendered_digest,
        json_media_type,
    )?;
    let artifact = artifacts.read_retained_artifact(&requirement).await?;
    let evidence = artifact.evidence().clone();
    verify_public_output_rendered_artifact_evidence(
        &evidence,
        artifact_id,
        rendered_digest,
        payload,
    )?;
    let bytes = artifact.into_bytes();
    serde_json::from_slice(&bytes).map_err(|error| {
        let _ = error;
        PublicError::backend(
            ErrorClass::Internal,
            "PublicOutputDecodeFailed",
            "Typed public-output artifact was not JSON",
        )
    })
}

pub(super) fn verify_public_output_rendered_artifact_evidence(
    evidence: &store::ArtifactEvidenceRef,
    artifact_id: &ArtifactId,
    rendered_digest: &ContentDigest,
    payload: &events::PublicOutputProduced,
) -> Result<(), PublicError> {
    let json_media_type = public_output_json_media_type()?;
    validate_artifact_requirement_for_app(
        public_output_rendered_artifact_requirement(
            payload,
            artifact_id,
            rendered_digest,
            json_media_type,
        )?,
        evidence,
        ErrorClass::Internal,
        "PublicOutputArtifactMismatch",
        "typed public-output cache artifact evidence does not match the produced event",
    )
}

fn public_output_json_media_type() -> Result<spec::MediaType, PublicError> {
    spec::MediaType::new("application/json").map_err(|_| {
        PublicError::backend(
            ErrorClass::Internal,
            "PublicOutputMediaTypeInvalid",
            "Public-output JSON media type is invalid",
        )
    })
}

pub(super) fn public_output_artifact_mismatch(message: &'static str) -> PublicError {
    PublicError::new(
        ErrorClass::Internal,
        "PublicOutputArtifactMismatch",
        message,
    )
}

pub(super) fn run_admitted_payload<'a>(
    run_id: &RunId,
    stream: &'a [store::KernelEventEnvelope],
) -> Result<&'a events::RunAdmitted, PublicError> {
    stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(payload.as_ref()),
            _ => None,
        })
        .ok_or_else(|| {
            PublicError::new(
                ErrorClass::Internal,
                "RunAdmittedMissing",
                "typed run stream is missing RunAdmitted evidence",
            )
        })
        .and_then(|run_admitted| {
            if &run_admitted.run_id == run_id {
                Ok(run_admitted)
            } else {
                Err(PublicError::new(
                    ErrorClass::Internal,
                    "RunAdmittedMismatch",
                    "typed run stream RunAdmitted evidence is bound to a different run id",
                ))
            }
        })
}

pub(super) fn validate_spec_artifact_evidence(
    run_admitted: &events::RunAdmitted,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<(), PublicError> {
    if run_admitted.spec_artifact.role != events::ArtifactRole::TypedExecutionSpec {
        return Err(PublicError::new(
            ErrorClass::Internal,
            "CertifiedSpecArtifactMismatch",
            "typed execution spec artifact metadata does not match RunAdmitted evidence",
        ));
    }
    validate_artifact_requirement_for_app(
        run_artifact_requirement(
            store::EventArtifactReferenceSource::RunSpec,
            &run_admitted.spec_artifact,
            events::ArtifactRole::TypedExecutionSpec,
        ),
        evidence,
        ErrorClass::Internal,
        "CertifiedSpecArtifactMismatch",
        "typed execution spec artifact metadata does not match RunAdmitted evidence",
    )?;
    let expected_spec_hash =
        SpecHash::from_digest(evidence.digest.algorithm(), *evidence.digest.digest());
    if expected_spec_hash != run_admitted.spec_hash {
        return Err(PublicError::new(
            ErrorClass::Internal,
            "CertifiedSpecArtifactMismatch",
            "typed execution spec artifact metadata does not match RunAdmitted evidence",
        ));
    }
    Ok(())
}

pub(super) fn validate_certificate_artifact_evidence(
    run_admitted: &events::RunAdmitted,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<(), PublicError> {
    if run_admitted.certificate_artifact.role != events::ArtifactRole::TypedSpecCertificate {
        return Err(PublicError::new(
            ErrorClass::Internal,
            "CertifiedCertificateArtifactMismatch",
            "typed spec certificate artifact metadata does not match RunAdmitted evidence",
        ));
    }
    validate_artifact_requirement_for_app(
        run_artifact_requirement(
            store::EventArtifactReferenceSource::RunCertificate,
            &run_admitted.certificate_artifact,
            events::ArtifactRole::TypedSpecCertificate,
        ),
        evidence,
        ErrorClass::Internal,
        "CertifiedCertificateArtifactMismatch",
        "typed spec certificate artifact metadata does not match RunAdmitted evidence",
    )?;
    Ok(())
}

pub(super) fn validate_run_admitted_matches_spec(
    run_admitted: &events::RunAdmitted,
    envelope: &spec::HashedSpecEnvelope,
) -> Result<(), PublicError> {
    if envelope.spec_hash != run_admitted.spec_hash
        || envelope.spec.media_type != run_admitted.spec_artifact.media_type
        || envelope.spec.spec_version != run_admitted.spec_version
        || envelope.spec.lowering_version != run_admitted.lowering_version
        || envelope.spec.public_outputs.public_schema_id != run_admitted.public_output_schema_id
        || envelope.spec.descriptor_identities != run_admitted.descriptor_identities
        || envelope
            .spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            != run_admitted.canonicalizer_identity
    {
        return Err(PublicError::new(
            ErrorClass::Internal,
            "RunAdmittedSpecMismatch",
            "RunAdmitted evidence does not match the stored certified spec artifact",
        ));
    }
    Ok(())
}

pub(super) fn run_response_from_projection(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projection: &store::ProjectionSnapshot,
    status: DriveStatus,
) -> Result<RunResponse, PublicError> {
    run_response_from_projection_with_spec_hash(
        run_id,
        runtime_spec,
        stream,
        projection,
        runtime_spec.spec_hash(),
        status.as_str(),
    )
}

pub(super) fn status_projection_from_verified_view_with_resource_lanes(
    view: &VerifiedRunHistoryView,
    global_projection: &store::ProjectionSnapshot,
) -> Result<store::ProjectionSnapshot, store::StoreError> {
    projection_with_resource_lanes(
        view.projection_snapshot(),
        global_projection
            .resource_lanes()
            .map(|(lane_key, projection)| (lane_key.clone(), projection.clone()))
            .collect(),
    )
}

pub(super) fn projection_with_resource_lanes(
    snapshot: &store::ProjectionSnapshot,
    resource_lanes: BTreeMap<store::ResourceLaneKey, store::ResourceLaneProjection>,
) -> Result<store::ProjectionSnapshot, store::StoreError> {
    let mut parts = store::ProjectionSnapshotParts::from_snapshot(snapshot);
    parts.resource_lanes = resource_lanes;
    store::ProjectionSnapshot::from_parts(parts)
}

pub(super) fn stream_head(stream: &[store::KernelEventEnvelope]) -> u64 {
    stream.last().map_or(0, |event| event.seq().as_u64())
}

pub(super) fn run_mode_status(mode: store::RunMode) -> RunModeStatus {
    mode.into()
}

pub(super) fn attempt_dispositions(
    projection: &store::ProjectionSnapshot,
) -> Vec<AttemptDispositionStatus> {
    projection
        .attempts()
        .map(|(_key, attempt)| attempt_disposition(attempt))
        .collect()
}

pub(crate) fn attempt_disposition(attempt: &store::AttemptProjection) -> AttemptDispositionStatus {
    let (disposition, attempt_no, retryable, error_code, output_cell_id) = match &attempt.status {
        store::AttemptStatus::Started { attempt_no, .. } => {
            ("started", Some(*attempt_no), None, None, None)
        }
        store::AttemptStatus::Completed { output_cell_id } => (
            "completed",
            None,
            None,
            None,
            Some(output_cell_id.as_str().to_owned()),
        ),
        store::AttemptStatus::Failed { retryable, error } => (
            "failed",
            None,
            Some(*retryable),
            Some(error.code.as_str().to_owned()),
            None,
        ),
        store::AttemptStatus::Interrupted => ("interrupted", None, None, None, None),
    };
    AttemptDispositionStatus {
        node_id: attempt.node_id.as_str().to_owned(),
        attempt_id: attempt.attempt_id.as_str().to_owned(),
        disposition: disposition.to_owned(),
        attempt_no,
        retryable,
        error_code,
        output_cell_id,
    }
}

pub(super) fn saga_status_with_resources(
    certified_spec: &spec::TypedExecutionSpec,
    projection: &store::ProjectionSnapshot,
    saga: &store::SagaProjection,
) -> SagaStatus {
    saga_status_inner(
        &certified_spec.saga,
        Some(certified_spec),
        Some(projection),
        saga,
    )
}

pub(super) fn saga_status_inner(
    policy: &spec::SagaPolicySpec,
    certified_spec: Option<&spec::TypedExecutionSpec>,
    projection: Option<&store::ProjectionSnapshot>,
    saga: &store::SagaProjection,
) -> SagaStatus {
    SagaStatus {
        policy: saga_policy_status(policy),
        obligations: saga
            .obligations
            .values()
            .map(|obligation| {
                obligation_status(&saga.run_id, obligation, certified_spec, projection)
            })
            .collect(),
        resource_ledgers: match (certified_spec, projection) {
            (Some(certified_spec), Some(projection)) => {
                resource_ledgers_for_run(certified_spec, projection, &saga.run_id)
            }
            _ => Vec::new(),
        },
        resource_lanes: projection
            .map(|projection| resource_lanes_for_run(projection, &saga.run_id))
            .unwrap_or_default(),
        manual_block_reason: saga.manual_block_reason.map(manual_block_reason_str),
        required_manual_authorization: matches!(saga.run_mode, store::RunMode::ManualBlocked)
            .then(|| manual_authorization_for_policy(policy))
            .flatten(),
        terminal_resolution: saga
            .run_completion
            .as_ref()
            .map(|completion| terminal_resolution_status(&completion.outcome)),
    }
}

pub(super) fn saga_policy_status(policy: &spec::SagaPolicySpec) -> SagaPolicyStatus {
    match policy {
        spec::SagaPolicySpec::NoSideEffects => SagaPolicyStatus {
            variant: "no_side_effects".to_owned(),
            manual_authorization: None,
            on_remediation_unresolved: None,
        },
        spec::SagaPolicySpec::FailWithoutAcdcClaim => SagaPolicyStatus {
            variant: "fail_without_acdc_claim".to_owned(),
            manual_authorization: None,
            on_remediation_unresolved: None,
        },
        spec::SagaPolicySpec::ManualResolution { manual } => SagaPolicyStatus {
            variant: "manual_resolution".to_owned(),
            manual_authorization: Some(manual_authorization_requirements(manual)),
            on_remediation_unresolved: None,
        },
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved,
        } => {
            let (directive, manual_authorization) = match on_remediation_unresolved {
                spec::RemediationUnresolvedSpec::ManualResolution { manual } => (
                    "manual_resolution",
                    Some(manual_authorization_requirements(manual)),
                ),
                spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim => {
                    ("fail_without_acdc_claim", None)
                }
            };
            SagaPolicyStatus {
                variant: "compensate_completed".to_owned(),
                manual_authorization,
                on_remediation_unresolved: Some(directive.to_owned()),
            }
        }
    }
}

pub(super) fn manual_authorization_for_policy(
    policy: &spec::SagaPolicySpec,
) -> Option<ManualAuthorizationRequirements> {
    match policy {
        spec::SagaPolicySpec::ManualResolution { manual } => {
            Some(manual_authorization_requirements(manual))
        }
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => Some(manual_authorization_requirements(manual)),
        spec::SagaPolicySpec::NoSideEffects
        | spec::SagaPolicySpec::FailWithoutAcdcClaim
        | spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim,
        } => None,
    }
}

pub(super) fn manual_authorization_requirements(
    manual: &spec::ManualResolutionEvidenceSpec,
) -> ManualAuthorizationRequirements {
    ManualAuthorizationRequirements {
        evidence_schema_id: manual.evidence_schema.as_str().to_owned(),
        verifier_id: manual.authorization.verifier_id.as_str().to_owned(),
        signing_scheme: manual.authorization.signing_scheme.as_str().to_owned(),
        authority_id: manual
            .authorization
            .authority
            .authority_id
            .as_str()
            .to_owned(),
        operator_public_identities: manual
            .authorization
            .authority
            .operators
            .iter()
            .map(|operator| operator.public_identity.as_str().to_owned())
            .collect(),
        quorum_required_signatures: manual.authorization.quorum.required_signatures(),
    }
}

pub(super) fn obligation_status(
    run_id: &RunId,
    obligation: &store::SagaObligationProjection,
    certified_spec: Option<&spec::TypedExecutionSpec>,
    projection: Option<&store::ProjectionSnapshot>,
) -> SagaObligationStatus {
    let forward_resource = projection
        .and_then(|projection| projection.side_effect_for_pair(run_id, &obligation.forward_pair_id))
        .and_then(|side_effect| resource_ledger_status(certified_spec?, projection?, side_effect));
    SagaObligationStatus {
        forward_ledger_key: obligation.forward_ledger_key.as_str().to_owned(),
        forward_phase: side_effect_phase_str(&obligation.forward_phase),
        classification: forward_classification_str(obligation.classification),
        resource: forward_resource,
        remediation: obligation.remediation.as_ref().map(|remediation| {
            let remediation_resource = projection
                .and_then(|projection| {
                    projection.side_effect_for_pair(run_id, &remediation.pair_id)
                })
                .and_then(|side_effect| {
                    resource_ledger_status(certified_spec?, projection?, side_effect)
                });
            RemediationLedgerStatus {
                ledger_key: remediation.ledger_key.as_str().to_owned(),
                forward_ledger_key: obligation.forward_ledger_key.as_str().to_owned(),
                phase: side_effect_phase_str(&remediation.phase),
                resource: remediation_resource,
                closed: remediation.closed,
                unresolved: remediation.unresolved.map(manual_block_reason_str),
            }
        }),
    }
}

pub(super) fn resource_ledger_status(
    certified_spec: &spec::TypedExecutionSpec,
    projection: &store::ProjectionSnapshot,
    side_effect: &store::SideEffectProjection,
) -> Option<ResourceLedgerStatus> {
    let node = certified_node(certified_spec, &side_effect.intent.node_id)?;
    let claim = &node.side_effect.as_ref()?.resource_claim;
    let key = side_effect.resource_key.as_ref().map(resource_key_status);
    let touched_set = side_effect
        .resource_touched_set
        .as_ref()
        .map(resource_touched_set_status);
    let (active_lane, blocked_by_lane) =
        if side_effect_phase_has_live_resource_lane_interest(&side_effect.phase) {
            side_effect
                .resource_key
                .as_ref()
                .and_then(|resource_key| {
                    let lane_key = store::ResourceLaneKey::from_evidence(resource_key);
                    projection
                        .resource_lane(&lane_key)
                        .map(|lane| (lane_key, lane))
                })
                .map(|(lane_key, lane)| {
                    let holder = resource_lane_holder_status(projection, &lane_key, lane);
                    let side_effect_ref = store::SideEffectPairLedgerRef::new(
                        side_effect.run_id.clone(),
                        side_effect.pair_id.clone(),
                    );
                    if lane.holder == side_effect_ref {
                        (Some(holder), None)
                    } else {
                        (None, Some(holder))
                    }
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };
    let ledger_purpose = ledger_purpose_status(&side_effect.ledger_purpose);
    let forward_ledger_key =
        forward_ledger_key_status(projection, &side_effect.run_id, &side_effect.ledger_purpose);

    Some(ResourceLedgerStatus {
        ledger_key: side_effect.ledger_key.as_str().to_owned(),
        ledger_purpose,
        forward_ledger_key,
        phase: side_effect_phase_str(&side_effect.phase),
        claim: resource_claim_status(claim),
        key,
        touched_set,
        active_lane,
        blocked_by_lane,
    })
}

pub(super) fn resource_ledgers_for_run(
    certified_spec: &spec::TypedExecutionSpec,
    projection: &store::ProjectionSnapshot,
    run_id: &RunId,
) -> Vec<ResourceLedgerStatus> {
    projection
        .side_effects()
        .filter_map(|(_, side_effect)| {
            if &side_effect.run_id == run_id {
                resource_ledger_status(certified_spec, projection, side_effect)
            } else {
                None
            }
        })
        .collect()
}

pub(super) fn resource_lanes_for_run(
    projection: &store::ProjectionSnapshot,
    run_id: &RunId,
) -> Vec<ResourceLaneHolderStatus> {
    let referenced_lane_keys = projection
        .side_effects()
        .filter_map(|(_, side_effect)| {
            if &side_effect.run_id != run_id {
                return None;
            }
            if !side_effect_phase_has_live_resource_lane_interest(&side_effect.phase) {
                return None;
            }
            side_effect
                .resource_key
                .as_ref()
                .map(store::ResourceLaneKey::from_evidence)
        })
        .collect::<BTreeSet<_>>();

    projection
        .resource_lanes()
        .filter(|(lane_key, _lane)| referenced_lane_keys.contains(*lane_key))
        .map(|(lane_key, lane)| resource_lane_holder_status(projection, lane_key, lane))
        .collect()
}

pub(super) fn side_effect_phase_has_live_resource_lane_interest(
    phase: &store::SideEffectPhase,
) -> bool {
    matches!(
        phase,
        store::SideEffectPhase::InvocationPrepared { .. }
            | store::SideEffectPhase::InvocationStarted { .. }
            | store::SideEffectPhase::SubmissionObserved { .. }
            | store::SideEffectPhase::SubmissionUnknown { .. }
            | store::SideEffectPhase::ReceiptObserved { .. }
    )
}

pub(super) fn resource_claim_status(claim: &spec::ResourceClaimSpec) -> ResourceClaimStatus {
    match claim {
        spec::ResourceClaimSpec::Exclusive {
            namespace,
            key_schema,
        } => ResourceClaimStatus {
            kind: "exclusive".to_owned(),
            namespace: Some(namespace.as_str().to_owned()),
            key_schema_id: Some(key_schema.as_str().to_owned()),
            evidence_schema_id: None,
        },
        spec::ResourceClaimSpec::ExactTouchedSet {
            namespace,
            evidence_schema,
        } => ResourceClaimStatus {
            kind: "exact_touched_set".to_owned(),
            namespace: Some(namespace.as_str().to_owned()),
            key_schema_id: None,
            evidence_schema_id: Some(evidence_schema.as_str().to_owned()),
        },
        spec::ResourceClaimSpec::ManualOnly => ResourceClaimStatus {
            kind: "manual_only".to_owned(),
            namespace: None,
            key_schema_id: None,
            evidence_schema_id: None,
        },
    }
}

pub(super) fn resource_key_status(evidence: &events::ResourceKeyEvidence) -> ResourceKeyStatus {
    ResourceKeyStatus {
        namespace: evidence.namespace.as_str().to_owned(),
        key_schema_id: evidence.key_schema_id.as_str().to_owned(),
        key_digest: resource_key_evidence_digest(evidence),
    }
}

pub(super) fn resource_touched_set_status(
    evidence: &events::ResourceTouchedSetEvidence,
) -> ResourceTouchedSetStatus {
    ResourceTouchedSetStatus {
        namespace: evidence.namespace.as_str().to_owned(),
        evidence_schema_id: evidence.evidence_schema_id.as_str().to_owned(),
        evidence_hash: evidence.evidence_hash.as_str().to_owned(),
        evidence_artifact_id: evidence.evidence_artifact_id.as_str().to_owned(),
    }
}

pub(super) fn resource_lane_holder_status(
    projection: &store::ProjectionSnapshot,
    lane_key: &store::ResourceLaneKey,
    lane: &store::ResourceLaneProjection,
) -> ResourceLaneHolderStatus {
    let holding_ledger_purpose = ledger_purpose_status(&lane.ledger_purpose);
    let holding_forward_ledger_key =
        forward_ledger_key_status(projection, &lane.holder.run_id, &lane.ledger_purpose);
    ResourceLaneHolderStatus {
        namespace: lane_key.namespace.as_str().to_owned(),
        key_schema_id: lane_key.key_schema_id.as_str().to_owned(),
        key_digest: resource_lane_key_digest(lane_key),
        holding_run_id: lane.holder.run_id.as_str().to_owned(),
        holding_ledger_key: lane.ledger_key.as_str().to_owned(),
        holding_ledger_purpose,
        holding_forward_ledger_key,
        holding_node_id: lane.node_id.as_str().to_owned(),
        holding_attempt_id: lane.attempt_id.as_str().to_owned(),
        invocation_epoch: lane.invocation_epoch,
    }
}

#[derive(Serialize)]
struct ResourceKeyDigestMaterial<'a> {
    key: &'a str,
    key_schema_id: &'a str,
    namespace: &'a str,
}

pub(super) fn resource_key_evidence_digest(evidence: &events::ResourceKeyEvidence) -> String {
    resource_key_digest_material(ResourceKeyDigestMaterial {
        key: evidence.key.as_str(),
        key_schema_id: evidence.key_schema_id.as_str(),
        namespace: evidence.namespace.as_str(),
    })
}

pub(super) fn resource_lane_key_digest(lane_key: &store::ResourceLaneKey) -> String {
    resource_key_digest_material(ResourceKeyDigestMaterial {
        key: lane_key.key.as_str(),
        key_schema_id: lane_key.key_schema_id.as_str(),
        namespace: lane_key.namespace.as_str(),
    })
}

fn resource_key_digest_material(material: ResourceKeyDigestMaterial<'_>) -> String {
    let json = serde_json::to_string(&material).expect("resource key digest material serializes");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .expect("resource key digest material is JSON");
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(canonical.as_bytes()),
    )
    .to_string()
}

pub(super) fn ledger_purpose_status(purpose: &events::SideEffectLedgerPurpose) -> String {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => "forward".to_owned(),
        events::SideEffectLedgerPurpose::Remediation { .. } => "remediation".to_owned(),
    }
}

pub(super) fn forward_ledger_key_status(
    projection: &store::ProjectionSnapshot,
    run_id: &RunId,
    purpose: &events::SideEffectLedgerPurpose,
) -> Option<String> {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => None,
        events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => projection
            .side_effect_for_pair(run_id, forward_pair_id)
            .map(|side_effect| side_effect.ledger_key.as_str().to_owned()),
    }
}

pub(super) fn certified_node<'a>(
    certified_spec: &'a spec::TypedExecutionSpec,
    node_id: &mfm_ids::NodeId,
) -> Option<&'a spec::NodeSpec> {
    certified_spec
        .nodes
        .iter()
        .chain(certified_spec.remediations.values())
        .find(|node| &node.node_id == node_id)
}

pub(super) fn terminal_resolution_status(
    outcome: &events::RunCompletionOutcome,
) -> TerminalResolutionStatus {
    TerminalResolutionStatus {
        outcome: store::codec::run_completion_outcome_str(outcome).to_owned(),
        claim: store::codec::run_completion_claim_str(outcome).to_owned(),
    }
}

pub(super) fn manual_block_reason_str(reason: store::ManualBlockReason) -> String {
    match reason {
        store::ManualBlockReason::PolicyManualResolution => "policy_manual_resolution",
        store::ManualBlockReason::ForwardAmbiguous => "forward_ambiguous",
        store::ManualBlockReason::RemediationFailed => "remediation_failed",
        store::ManualBlockReason::RemediationAmbiguous => "remediation_ambiguous",
    }
    .to_owned()
}

pub(super) fn forward_classification_str(
    classification: store::ForwardLedgerClassification,
) -> String {
    match classification {
        store::ForwardLedgerClassification::Pending => "pending",
        store::ForwardLedgerClassification::NothingOwed => "nothing_owed",
        store::ForwardLedgerClassification::Owed => "owed",
        store::ForwardLedgerClassification::Unresolvable => "unresolvable",
    }
    .to_owned()
}

pub(super) fn side_effect_phase_str(phase: &store::SideEffectPhase) -> String {
    phase.as_str().to_owned()
}

pub(super) fn run_admitted_spec_hash(
    stream: &[store::KernelEventEnvelope],
) -> Result<SpecHash, PublicError> {
    stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(payload.spec_hash.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            PublicError::new(
                ErrorClass::Internal,
                "RunAdmittedMissing",
                "typed run stream is missing RunAdmitted evidence",
            )
        })
}

pub(super) fn scheduler_status_str(status: SchedulerStatus) -> &'static str {
    match status {
        SchedulerStatus::Advanced => "advanced",
        SchedulerStatus::Blocked => "blocked",
        SchedulerStatus::PublicOutputProjected => "public_output_projected",
    }
}

pub(crate) fn run_event_ref(event: &store::KernelEventEnvelope) -> RunEventRef {
    RunEventRef {
        event_id: event.event_id().as_str().to_owned(),
        event_schema_id: event.event_schema_id().as_str().to_owned(),
        seq: event.seq().as_u64(),
        ordinal: event.ordinal().as_u32(),
        commit_key: event.commit_key().as_str().to_owned(),
        logical_key: event.logical_key().as_str().to_owned(),
        payload_hash: event.payload_hash().as_str().to_owned(),
        error_code: match event.payload() {
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                Some(payload.error.code.as_str().to_owned())
            }
            _ => None,
        },
    }
}
