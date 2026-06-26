use super::*;

pub(super) fn require_admission_preconditions(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    payload: &KernelEventPayload,
    certified_run_authority: Option<&CertifiedRunStoreAuthority>,
) -> Result<()> {
    if payload.side_effect_ledger_ref().is_some() {
        let authority = require_certified_run_authority(
            projections,
            run_id,
            payload.spec_hash(),
            certified_run_authority,
        )?;
        require_side_effect_payload_authority(projections, run_id, payload, authority)?;
    }

    match payload {
        KernelEventPayload::ManualResolutionRecorded(payload) => {
            let authority = require_certified_run_authority(
                projections,
                &payload.run_id,
                &payload.spec_hash,
                certified_run_authority,
            )?;
            projections.require_manual_resolution_admissible(
                &payload.run_id,
                authority.saga_policy(),
                authority.terminal_policies(),
            )
        }
        KernelEventPayload::RunCompleted(payload) if certified_run_authority.is_some() => {
            let authority = require_certified_run_authority(
                projections,
                &payload.run_id,
                &payload.spec_hash,
                certified_run_authority,
            )?;
            let projected_outcome = projections.saga_terminal_completion_outcome(
                &payload.run_id,
                authority.saga_policy(),
                authority.terminal_policies(),
            )?;
            if projected_outcome != payload.outcome {
                return Err(StoreError::ProjectionConflict {
                    key: format!("run:{}:saga_terminal", payload.run_id),
                    message: "RunCompleted outcome does not match current saga projection"
                        .to_owned(),
                });
            }
            Ok(())
        }
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            if !matches!(
                payload.ledger_purpose,
                events::SideEffectLedgerPurpose::Remediation { .. }
            ) {
                return Ok(());
            }
            let token = require_certified_run_authority(
                projections,
                run_id,
                &payload.spec_hash,
                certified_run_authority,
            )?;
            require_remediation_intent_admissible(
                projections,
                run_id,
                &payload.ledger_key,
                &payload.ledger_purpose,
                token.terminal_policies(),
            )
        }
        _ => Ok(()),
    }
}

fn require_side_effect_payload_authority(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    payload: &KernelEventPayload,
    authority: &CertifiedRunStoreAuthority,
) -> Result<()> {
    let ledger =
        payload
            .side_effect_ledger_ref()
            .ok_or_else(|| StoreError::ProjectionConflict {
                key: format!("run:{run_id}:side_effect_authority"),
                message: "side-effect payload authority is missing".to_owned(),
            })?;
    if authority.spec_hash() != payload.spec_hash() {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:side_effect_authority"),
            message: "side-effect payload spec hash does not match certified authority".to_owned(),
        });
    }
    let pair = authority.side_effect_pair(ledger.pair_id)?;
    if pair.pair_id != *ledger.pair_id {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx_pair:{}", ledger.pair_id),
            message: "certified side-effect pair id mismatch".to_owned(),
        });
    }
    if let Some(release_ledger) = payload
        .resource_lane_authority_ref()
        .and_then(|resource| resource.release_authority.map(|_| resource.ledger))
    {
        if release_ledger.pair_role != events::SideEffectPairRole::Verify {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx_pair:{}", release_ledger.pair_id),
                message: "resource lane release requires verify pair role".to_owned(),
            });
        }
        return Ok(());
    }

    let emitter =
        payload
            .side_effect_emitter_ref()
            .ok_or_else(|| StoreError::ProjectionConflict {
                key: format!("sidefx_pair:{}", ledger.pair_id),
                message: "side-effect payload requires certified emitter attribution".to_owned(),
            })?;
    if emitter.pair_role != ledger.pair_role {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx_pair:{}", ledger.pair_id),
            message: "side-effect emitter role does not match ledger role".to_owned(),
        });
    }
    let expected_node = pair.node_for_role(ledger.pair_role);
    if emitter.node_id != expected_node {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx_pair:{}", ledger.pair_id),
            message: "side-effect emitter node is not certified for pair role".to_owned(),
        });
    }
    require_active_attempt_for_side_effect(
        projections,
        emitter.node_id,
        emitter.attempt_id,
        ledger.ledger_key,
    )
}

fn require_certified_run_authority<'a>(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    spec_hash: &SpecHash,
    certified_run_authority: Option<&'a CertifiedRunStoreAuthority>,
) -> Result<&'a CertifiedRunStoreAuthority> {
    let token = certified_run_authority.ok_or_else(|| StoreError::ProjectionConflict {
        key: format!("run:{run_id}:saga_policy"),
        message: "certified run authority is required for saga admission".to_owned(),
    })?;
    if token.run_id() != run_id || token.spec_hash() != spec_hash {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:saga_policy"),
            message: "certified run authority run or spec hash does not match payload".to_owned(),
        });
    }
    let Some(projected_spec_hash) = projections.run_spec_hash(run_id) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:saga_policy"),
            message: "run-start spec hash is not projected".to_owned(),
        });
    };
    if projected_spec_hash != token.spec_hash() {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:saga_policy"),
            message: "certified run authority spec hash does not match run start".to_owned(),
        });
    }
    let Some(projected_digest) = projections.saga_policy_digest(run_id) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:saga_policy"),
            message: "run-start saga policy digest is not projected".to_owned(),
        });
    };
    if projected_digest != token.saga_policy_digest() {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:saga_policy"),
            message: "certified run authority digest does not match run start".to_owned(),
        });
    }
    Ok(token)
}
