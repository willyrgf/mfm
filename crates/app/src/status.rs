use super::*;
use mfm_store::v1::current_lifecycle::{
    CurrentAttemptRef, CurrentAttemptStatusRef, CurrentLifecycleReader, CurrentSagaObligationRef,
    CurrentSagaRef, CurrentSideEffectRef,
};

pub(super) fn run_response_from_verified_status(
    context: &VerifiedStatusReadContext,
    scheduler_status: &str,
) -> Result<RunResponse, PublicError> {
    run_response_from_verified_current(
        context.run(),
        context.resource_lane_projection(),
        scheduler_status,
    )
}

pub(super) fn run_response_from_verified_current(
    current: &VerifiedCurrentRun,
    resource_lane_projection: &store::ProjectionSnapshot,
    scheduler_status: &str,
) -> Result<RunResponse, PublicError> {
    let view = current.view();
    let lifecycle = store::current_lifecycle::read(view);
    let certified_spec = lifecycle.certified_spec().validated_spec().spec();
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(certified_spec)?;
    lifecycle
        .with_saga(&certified_spec.saga, &terminal_policies, |saga| {
            RunResponse {
                run_id: view.run_id().as_str().to_owned(),
                spec_hash: view.spec_hash().as_str().to_owned(),
                run_mode: run_mode_status(saga.run_mode()),
                saga: saga_status_with_resources(
                    certified_spec,
                    &lifecycle,
                    Some(resource_lane_projection),
                    &saga,
                ),
                attempt_dispositions: attempt_dispositions(&lifecycle),
                scheduler_status: scheduler_status.to_owned(),
                head_seq: view.current_run_sequence().unwrap_or_default(),
            }
        })
        .map_err(PublicError::from)
}

pub(super) fn run_stream_response_from_verified_context(
    context: &VerifiedRunReadContext,
) -> RunStreamResponse {
    let lifecycle = context.lifecycle();
    let mut events = Vec::new();
    let _ = lifecycle.visit_records(|record| {
        events.push(run_event_ref(record));
        std::ops::ControlFlow::<()>::Continue(())
    });
    RunStreamResponse {
        run_id: context.view().run_id().as_str().to_owned(),
        head_seq: context.view().current_run_sequence().unwrap_or(0),
        events,
    }
}

/// Builds typed public-output read authority by borrowing one verified run view.
pub fn public_output_read_authority_for_run<'view>(
    verified_view: &'view store::VerifiedRunView,
    public_schema_id: &SchemaId,
) -> Result<PublicOutputReadAuthority<'view>, PublicError> {
    let lifecycle = store::current_lifecycle::read(verified_view);
    let certified_spec = lifecycle.certified_spec().validated_spec().spec();
    let public_output = lifecycle.public_output(public_schema_id).ok_or_else(|| {
        PublicError::not_found(
            "PublicOutputNotFound",
            "typed public output was not found for the requested schema",
        )
    })?;
    let Some(produced) = public_output.produced() else {
        return Err(PublicError::new(
            ErrorClass::Conflict,
            "PublicOutputRenderFailed",
            "typed public output render failed",
        ));
    };

    let event_id = public_output.event_id();
    let payload = public_output_payload_from_view(verified_view, event_id, public_schema_id)?;
    if &payload.spec_hash != verified_view.spec_hash()
        || &payload.public_schema_id != public_schema_id
        || public_schema_id != &certified_spec.public_outputs.public_schema_id
    {
        return Err(PublicError::new(
            ErrorClass::Internal,
            "PublicOutputAuthorityMismatch",
            "typed public-output evidence does not match certified runtime authority",
        ));
    }
    verify_public_output_authority_artifacts(
        verified_view,
        payload,
        produced.rendered_artifact_id(),
        produced.rendered_digest(),
    )?;

    Ok(PublicOutputReadAuthority {
        run_id: verified_view.run_id(),
        public_schema_id: &payload.public_schema_id,
        event_id,
        rendered_digest: produced.rendered_digest(),
        rendered_artifact_id: produced.rendered_artifact_id(),
        payload,
    })
}

pub(super) fn verify_public_output_authority_artifacts(
    view: &store::VerifiedRunView,
    payload: &events::PublicOutputProduced,
    rendered_artifact_id: Option<&ArtifactId>,
    rendered_digest: &ContentDigest,
) -> Result<(), PublicError> {
    let lifecycle = store::current_lifecycle::read(view);
    for cell in &payload.cells {
        let requirement = store::public_output_cell_artifact_requirement(cell);
        let artifact = lifecycle
            .object_for_requirement(&requirement)
            .ok_or_else(|| {
                public_output_artifact_mismatch(
                    "typed public-output cell object is missing from verified history",
                )
            })?;
        verify_public_output_cell_evidence(cell, artifact.evidence())?;
    }
    if let Some(artifact_id) = rendered_artifact_id {
        let json_media_type = public_output_json_media_type()?;
        let requirement = store::public_output_rendered_artifact_requirement(
            payload,
            artifact_id,
            rendered_digest,
            json_media_type,
        )?;
        let artifact = lifecycle
            .object_for_requirement(&requirement)
            .ok_or_else(|| {
                public_output_artifact_mismatch(
                    "typed public-output rendered object is missing from verified history",
                )
            })?;
        verify_public_output_rendered_artifact_evidence(
            artifact.evidence(),
            artifact_id,
            rendered_digest,
            payload,
        )?;
    }
    Ok(())
}

/// Renders typed public output from one verified view and scoped read authority.
pub fn render_public_output(
    view: &store::VerifiedRunView,
    authority: &PublicOutputReadAuthority<'_>,
) -> Result<PublicOutputResponse, PublicError> {
    if authority.run_id() != view.run_id() {
        return Err(PublicError::new(
            ErrorClass::Internal,
            "PublicOutputAuthorityMismatch",
            "typed public-output authority belongs to a different verified run",
        ));
    }
    let json = match authority.rendered_artifact_id() {
        Some(artifact_id) => Some(load_public_output_json(
            view,
            artifact_id,
            authority.rendered_digest(),
            authority.payload,
        )?),
        None => Some(render_public_output_json_from_authority(view, authority)?),
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

pub(super) fn public_output_payload_from_view<'a>(
    view: &'a store::VerifiedRunView,
    event_id: &EventId,
    public_schema_id: &SchemaId,
) -> Result<&'a events::PublicOutputProduced, PublicError> {
    let lifecycle = store::current_lifecycle::read(view);
    let mut payload = None;
    let _ = lifecycle.visit_records(|record| {
        if record.event_id() == event_id {
            if let store::current_lifecycle::CurrentRecordKindRef::PublicOutputProduced(candidate) =
                record.kind()
            {
                if &candidate.public_schema_id == public_schema_id {
                    payload = Some(candidate);
                }
            }
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    payload.ok_or_else(|| {
        PublicError::new(
            ErrorClass::Internal,
            "PublicOutputProjectionMismatch",
            "typed public-output projection does not match the verified journal",
        )
    })
}

pub(super) fn render_public_output_json_from_authority(
    view: &store::VerifiedRunView,
    authority: &PublicOutputReadAuthority<'_>,
) -> Result<Value, PublicError> {
    let lifecycle = store::current_lifecycle::read(view);
    let mut root = Map::new();
    for cell in &authority.payload.cells {
        let requirement = store::public_output_cell_artifact_requirement(cell);
        let artifact = lifecycle
            .object_for_requirement(&requirement)
            .ok_or_else(|| {
                public_output_artifact_mismatch(
                    "typed public-output cell object is missing from verified history",
                )
            })?;
        verify_public_output_cell_evidence(cell, artifact.evidence())?;
        let value = serde_json::from_slice(artifact.bytes()).map_err(|error| {
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
        store::public_output_cell_artifact_requirement(cell),
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
                    let Some(value) = value.take() else {
                        return Err(public_output_artifact_mismatch(
                            "typed public-output field value was consumed more than once",
                        ));
                    };
                    entry.insert(value);
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

pub(super) fn load_public_output_json(
    view: &store::VerifiedRunView,
    artifact_id: &ArtifactId,
    rendered_digest: &mfm_ids::ContentDigest,
    payload: &events::PublicOutputProduced,
) -> Result<serde_json::Value, PublicError> {
    let json_media_type = public_output_json_media_type()?;
    let requirement = store::public_output_rendered_artifact_requirement(
        payload,
        artifact_id,
        rendered_digest,
        json_media_type,
    )?;
    let artifact = store::current_lifecycle::read(view)
        .object_for_requirement(&requirement)
        .ok_or_else(|| {
            public_output_artifact_mismatch(
                "typed public-output rendered object is missing from verified history",
            )
        })?;
    verify_public_output_rendered_artifact_evidence(
        artifact.evidence(),
        artifact_id,
        rendered_digest,
        payload,
    )?;
    serde_json::from_slice(artifact.bytes()).map_err(|error| {
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
        store::public_output_rendered_artifact_requirement(
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

pub(super) fn run_mode_status(mode: store::RunMode) -> RunModeStatus {
    mode.into()
}

pub(super) fn attempt_dispositions(
    lifecycle: &CurrentLifecycleReader<'_>,
) -> Vec<AttemptDispositionStatus> {
    let mut dispositions = Vec::new();
    let _ = lifecycle.visit_attempts(|attempt| {
        dispositions.push(attempt_disposition(attempt));
        std::ops::ControlFlow::<()>::Continue(())
    });
    dispositions
}

pub(crate) fn attempt_disposition(attempt: CurrentAttemptRef<'_>) -> AttemptDispositionStatus {
    let (disposition, attempt_no, retryable, error_code, output_cell_id) = match attempt.status() {
        CurrentAttemptStatusRef::Started { attempt_no, .. } => {
            ("started", Some(attempt_no), None, None, None)
        }
        CurrentAttemptStatusRef::Completed { output_cell_id } => (
            "completed",
            None,
            None,
            None,
            Some(output_cell_id.as_str().to_owned()),
        ),
        CurrentAttemptStatusRef::Failed { retryable, error } => (
            "failed",
            None,
            Some(retryable),
            Some(error.code.as_str().to_owned()),
            None,
        ),
        CurrentAttemptStatusRef::Interrupted => ("interrupted", None, None, None, None),
    };
    AttemptDispositionStatus {
        node_id: attempt.node_id().as_str().to_owned(),
        attempt_id: attempt.attempt_id().as_str().to_owned(),
        disposition: disposition.to_owned(),
        attempt_no,
        retryable,
        error_code,
        output_cell_id,
    }
}

pub(super) fn saga_status_with_resources(
    certified_spec: &spec::TypedExecutionSpec,
    lifecycle: &CurrentLifecycleReader<'_>,
    resource_lane_projection: Option<&store::ProjectionSnapshot>,
    saga: &CurrentSagaRef,
) -> SagaStatus {
    let mut obligations = Vec::new();
    let _ = saga.visit_obligations(|obligation| {
        obligations.push(obligation_status(
            obligation,
            certified_spec,
            lifecycle,
            resource_lane_projection,
        ));
        std::ops::ControlFlow::<()>::Continue(())
    });
    SagaStatus {
        policy: saga_policy_status(&certified_spec.saga),
        obligations,
        resource_ledgers: resource_ledgers_for_run(
            certified_spec,
            lifecycle,
            resource_lane_projection,
        ),
        resource_lanes: resource_lanes_for_run(lifecycle, resource_lane_projection),
        manual_block_reason: saga.manual_block_reason().map(manual_block_reason_str),
        required_manual_authorization: matches!(saga.run_mode(), store::RunMode::ManualBlocked)
            .then(|| manual_authorization_for_policy(&certified_spec.saga))
            .flatten(),
        terminal_resolution: saga
            .completion()
            .map(|completion| terminal_resolution_status(completion.outcome())),
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
    obligation: CurrentSagaObligationRef<'_>,
    certified_spec: &spec::TypedExecutionSpec,
    lifecycle: &CurrentLifecycleReader<'_>,
    resource_lane_projection: Option<&store::ProjectionSnapshot>,
) -> SagaObligationStatus {
    let forward_resource = lifecycle
        .side_effect(obligation.forward_pair_id())
        .and_then(|side_effect| {
            resource_ledger_status(
                certified_spec,
                lifecycle,
                resource_lane_projection,
                side_effect,
            )
        });
    SagaObligationStatus {
        forward_ledger_key: obligation.forward_ledger_key().as_str().to_owned(),
        forward_phase: side_effect_phase_str(obligation.forward_phase()),
        classification: forward_classification_str(obligation.classification()),
        resource: forward_resource,
        remediation: obligation.remediation().map(|remediation| {
            let remediation_resource =
                lifecycle
                    .side_effect(remediation.pair_id())
                    .and_then(|side_effect| {
                        resource_ledger_status(
                            certified_spec,
                            lifecycle,
                            resource_lane_projection,
                            side_effect,
                        )
                    });
            RemediationLedgerStatus {
                ledger_key: remediation.ledger_key().as_str().to_owned(),
                forward_ledger_key: obligation.forward_ledger_key().as_str().to_owned(),
                phase: side_effect_phase_str(remediation.phase()),
                resource: remediation_resource,
                closed: remediation.closed(),
                unresolved: remediation.unresolved().map(manual_block_reason_str),
            }
        }),
    }
}

pub(super) fn resource_ledger_status(
    certified_spec: &spec::TypedExecutionSpec,
    lifecycle: &CurrentLifecycleReader<'_>,
    resource_lane_projection: Option<&store::ProjectionSnapshot>,
    side_effect: CurrentSideEffectRef<'_>,
) -> Option<ResourceLedgerStatus> {
    let node = certified_node(certified_spec, side_effect.intent().node_id())?;
    let claim = &node.side_effect.as_ref()?.resource_claim;
    let key = side_effect.resource_key().map(resource_key_status);
    let touched_set = side_effect
        .resource_touched_set()
        .map(resource_touched_set_status);
    let (active_lane, blocked_by_lane) =
        if side_effect_phase_has_live_resource_lane_interest(side_effect.phase()) {
            if let Some(projection) = resource_lane_projection {
                side_effect
                    .resource_key()
                    .and_then(|resource_key| {
                        let lane_key = store::ResourceLaneKey::from_evidence(resource_key);
                        projection
                            .resource_lane(&lane_key)
                            .map(|lane| (lane_key, lane))
                    })
                    .map(|(lane_key, lane)| {
                        let holder = resource_lane_holder_status(projection, &lane_key, lane);
                        if &lane.holder.run_id == side_effect.run_id()
                            && &lane.holder.pair_id == side_effect.pair_id()
                        {
                            (Some(holder), None)
                        } else {
                            (None, Some(holder))
                        }
                    })
                    .unwrap_or((None, None))
            } else {
                side_effect
                    .resource_key()
                    .and_then(|resource_key| {
                        let lane_key = store::ResourceLaneKey::from_evidence(resource_key);
                        let lane = lifecycle.resource_lane(&lane_key)?;
                        Some((lane_key, lane))
                    })
                    .map(|(_lane_key, lane)| {
                        let holder = current_resource_lane_holder_status(lifecycle, &lane);
                        if lane.holder().run_id() == side_effect.run_id()
                            && lane.holder().pair_id() == side_effect.pair_id()
                        {
                            (Some(holder), None)
                        } else {
                            (None, Some(holder))
                        }
                    })
                    .unwrap_or((None, None))
            }
        } else {
            (None, None)
        };
    let ledger_purpose = ledger_purpose_status(side_effect.ledger_purpose());
    let forward_ledger_key =
        current_forward_ledger_key_status(lifecycle, side_effect.ledger_purpose());

    Some(ResourceLedgerStatus {
        ledger_key: side_effect.ledger_key().as_str().to_owned(),
        ledger_purpose,
        forward_ledger_key,
        phase: side_effect_phase_str(side_effect.phase()),
        claim: resource_claim_status(claim),
        key,
        touched_set,
        active_lane,
        blocked_by_lane,
    })
}

pub(super) fn resource_ledgers_for_run(
    certified_spec: &spec::TypedExecutionSpec,
    lifecycle: &CurrentLifecycleReader<'_>,
    resource_lane_projection: Option<&store::ProjectionSnapshot>,
) -> Vec<ResourceLedgerStatus> {
    let mut statuses = Vec::new();
    let _ = lifecycle.visit_side_effects(|side_effect| {
        if let Some(status) = resource_ledger_status(
            certified_spec,
            lifecycle,
            resource_lane_projection,
            side_effect,
        ) {
            statuses.push(status);
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    statuses
}

pub(super) fn resource_lanes_for_run(
    lifecycle: &CurrentLifecycleReader<'_>,
    resource_lane_projection: Option<&store::ProjectionSnapshot>,
) -> Vec<ResourceLaneHolderStatus> {
    let mut referenced_lane_keys = BTreeSet::new();
    let _ = lifecycle.visit_side_effects(|side_effect| {
        if side_effect_phase_has_live_resource_lane_interest(side_effect.phase()) {
            if let Some(resource_key) = side_effect.resource_key() {
                referenced_lane_keys.insert(store::ResourceLaneKey::from_evidence(resource_key));
            }
        }
        std::ops::ControlFlow::<()>::Continue(())
    });

    if let Some(resource_lane_projection) = resource_lane_projection {
        return resource_lane_projection
            .resource_lanes()
            .filter(|(lane_key, _lane)| referenced_lane_keys.contains(*lane_key))
            .map(|(lane_key, lane)| {
                resource_lane_holder_status(resource_lane_projection, lane_key, lane)
            })
            .collect();
    }
    let mut statuses = Vec::new();
    let _ = lifecycle.visit_resource_lanes(|lane| {
        if referenced_lane_keys.contains(lane.key()) {
            statuses.push(current_resource_lane_holder_status(lifecycle, &lane));
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    statuses
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

fn current_resource_lane_holder_status(
    lifecycle: &CurrentLifecycleReader<'_>,
    lane: &store::current_lifecycle::CurrentResourceLaneRef<'_>,
) -> ResourceLaneHolderStatus {
    let holder = lane.holder();
    ResourceLaneHolderStatus {
        namespace: lane.key().namespace.as_str().to_owned(),
        key_schema_id: lane.key().key_schema_id.as_str().to_owned(),
        key_digest: resource_lane_key_digest(lane.key()),
        holding_run_id: holder.run_id().as_str().to_owned(),
        holding_ledger_key: lane.ledger_key().as_str().to_owned(),
        holding_ledger_purpose: ledger_purpose_status(lane.ledger_purpose()),
        holding_forward_ledger_key: current_forward_ledger_key_status(
            lifecycle,
            lane.ledger_purpose(),
        ),
        holding_node_id: lane.node_id().as_str().to_owned(),
        holding_attempt_id: lane.attempt_id().as_str().to_owned(),
        invocation_epoch: lane.invocation_epoch(),
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

fn current_forward_ledger_key_status(
    lifecycle: &CurrentLifecycleReader<'_>,
    purpose: &events::SideEffectLedgerPurpose,
) -> Option<String> {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => None,
        events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => lifecycle
            .side_effect(forward_pair_id)
            .map(|side_effect| side_effect.ledger_key().as_str().to_owned()),
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

pub(super) fn scheduler_status_str(status: SchedulerStatus) -> &'static str {
    match status {
        SchedulerStatus::Advanced => "advanced",
        SchedulerStatus::Blocked => "blocked",
        SchedulerStatus::PublicOutputProjected => "public_output_projected",
    }
}

pub(crate) fn run_event_ref(record: store::current_lifecycle::CurrentRecordRef<'_>) -> RunEventRef {
    RunEventRef {
        event_id: record.event_id().as_str().to_owned(),
        event_schema_id: record.event_schema_id().as_str().to_owned(),
        seq: record.sequence().as_u64(),
        ordinal: record.ordinal().as_u32(),
        commit_key: record.commit_key().as_str().to_owned(),
        logical_key: record.logical_key().as_str().to_owned(),
        payload_hash: record.payload_hash().as_str().to_owned(),
        error_code: match record.kind() {
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptFailed(payload) => {
                Some(payload.error.code.as_str().to_owned())
            }
            _ => None,
        },
    }
}
