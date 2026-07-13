use super::*;

#[derive(Debug, Clone)]
pub(crate) struct OwnedSideEffectLedgerState {
    core: SideEffectLedgerCore,
    retained: SideEffectLedgerRetained,
    phase: OwnedSideEffectLedgerPhase,
}

struct SubmissionResultContext<'a> {
    event_id: EventId,
    ledger_key: &'a events::SideEffectLedgerKey,
    ledger_purpose: &'a events::SideEffectLedgerPurpose,
    pair_role: events::SideEffectPairRole,
    node_id: &'a NodeId,
    attempt_id: &'a AttemptId,
    invocation_epoch: u32,
}

#[derive(Debug, Clone)]
struct SideEffectLedgerCore {
    run_id: RunId,
    ledger_key: events::SideEffectLedgerKey,
    ledger_purpose: events::SideEffectLedgerPurpose,
    pair_id: SideEffectPairId,
    event_id: EventId,
    intent: SideEffectIntentProjection,
}

#[derive(Debug, Clone, Default)]
struct SideEffectLedgerRetained {
    prepared_invocation: Option<SideEffectArtifactProjection>,
    resource_key: Option<events::ResourceKeyEvidence>,
    submission: Option<SideEffectArtifactProjection>,
    receipt: Option<SideEffectArtifactProjection>,
    confirmation: Option<SideEffectArtifactProjection>,
    resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
}

#[derive(Debug, Clone)]
enum OwnedSideEffectLedgerPhase {
    IntentPersisted {
        invocation_epoch: u32,
    },
    Claimed {
        claim: SideEffectClaimProjection,
    },
    Prepared {
        claim: SideEffectClaimProjection,
    },
    Started {
        claim: SideEffectClaimProjection,
    },
    SubmissionObserved {
        claim: SideEffectClaimProjection,
    },
    NotSubmitted {
        claim: SideEffectClaimProjection,
    },
    SubmissionUnknown {
        claim: SideEffectClaimProjection,
    },
    ReceiptObserved {
        claim: SideEffectClaimProjection,
    },
    Confirmed {
        claim: SideEffectClaimProjection,
    },
    Ambiguous {
        claim: SideEffectClaimProjection,
        invocation_epoch: u32,
    },
    Failed {
        claim: Option<SideEffectClaimProjection>,
        invocation_epoch: u32,
        failure_phase: side_effect::FailurePhase,
    },
}

impl OwnedSideEffectLedgerState {
    pub(crate) fn intent_persisted(
        run_id: RunId,
        ledger_key: events::SideEffectLedgerKey,
        ledger_purpose: events::SideEffectLedgerPurpose,
        pair_id: SideEffectPairId,
        event_id: EventId,
        intent: SideEffectIntentProjection,
        invocation_epoch: u32,
    ) -> Self {
        Self {
            core: SideEffectLedgerCore {
                run_id,
                ledger_key,
                ledger_purpose,
                pair_id,
                event_id,
                intent,
            },
            retained: SideEffectLedgerRetained::default(),
            phase: OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch },
        }
    }

    pub(crate) fn from_projection(projection: SideEffectProjection) -> Result<Self> {
        projection.ledger_state()?;
        let core = SideEffectLedgerCore {
            run_id: projection.run_id,
            ledger_key: projection.ledger_key,
            ledger_purpose: projection.ledger_purpose,
            pair_id: projection.pair_id,
            event_id: projection.event_id,
            intent: projection.intent,
        };
        let retained = SideEffectLedgerRetained {
            prepared_invocation: projection.prepared_invocation,
            resource_key: projection.resource_key,
            submission: projection.submission,
            receipt: projection.receipt,
            confirmation: projection.confirmation,
            resource_touched_set: projection.resource_touched_set,
        };
        let phase = match projection.phase {
            SideEffectPhase::IntentPersisted { invocation_epoch } => {
                OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch }
            }
            SideEffectPhase::Claimed { .. } => OwnedSideEffectLedgerPhase::Claimed {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::InvocationPrepared { .. } => OwnedSideEffectLedgerPhase::Prepared {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::InvocationStarted { .. } => OwnedSideEffectLedgerPhase::Started {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::SubmissionObserved { .. } => {
                OwnedSideEffectLedgerPhase::SubmissionObserved {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::NotSubmittedProven { .. } => {
                OwnedSideEffectLedgerPhase::NotSubmitted {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::SubmissionUnknown { .. } => {
                OwnedSideEffectLedgerPhase::SubmissionUnknown {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::ReceiptObserved { .. } => {
                OwnedSideEffectLedgerPhase::ReceiptObserved {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::ConfirmationObserved { .. } => OwnedSideEffectLedgerPhase::Confirmed {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::Ambiguous { invocation_epoch } => {
                OwnedSideEffectLedgerPhase::Ambiguous {
                    claim: required_owned_claim(projection.claim)?,
                    invocation_epoch,
                }
            }
            SideEffectPhase::Failed {
                invocation_epoch,
                failure_phase,
            } => OwnedSideEffectLedgerPhase::Failed {
                claim: projection.claim,
                invocation_epoch,
                failure_phase,
            },
        };
        Ok(Self {
            core,
            retained,
            phase,
        })
    }

    pub(crate) fn into_projection(self) -> SideEffectProjection {
        let (claim, phase) = self.phase.into_projection_parts();
        SideEffectProjection {
            run_id: self.core.run_id,
            ledger_key: self.core.ledger_key,
            ledger_purpose: self.core.ledger_purpose,
            pair_id: self.core.pair_id,
            event_id: self.core.event_id,
            intent: self.core.intent,
            prepared_invocation: self.retained.prepared_invocation,
            resource_key: self.retained.resource_key,
            submission: self.retained.submission,
            receipt: self.retained.receipt,
            confirmation: self.retained.confirmation,
            resource_touched_set: self.retained.resource_touched_set,
            claim,
            phase,
        }
    }

    pub(crate) fn claim(
        mut self,
        event_id: EventId,
        payload: &side_effect::Claimed,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let next_claim = match &self.phase {
            OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch } => {
                if payload.invocation_epoch != *invocation_epoch {
                    return Err(self.error("initial claim invocation epoch does not match intent"));
                }
                self.require_intent_context(
                    &payload.node_id,
                    &payload.attempt_id,
                    payload.invocation_epoch,
                )?;
                side_effect_claim_projection(
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.claim_owner,
                    payload.invocation_epoch,
                    payload.claim_generation,
                    &payload.claim_fencing_token,
                )
            }
            OwnedSideEffectLedgerPhase::NotSubmitted { claim } => {
                self.require_intent_attempt_context(&payload.node_id, &payload.attempt_id)?;
                if payload.claim_generation <= claim.claim_generation {
                    return Err(self.error("retry claim generation must increase"));
                }
                if payload.claim_fencing_token == claim.claim_fencing_token {
                    return Err(self.error("retry claim fencing token must change"));
                }
                let next_epoch = claim
                    .invocation_epoch
                    .checked_add(1)
                    .ok_or_else(|| self.error("invocation epoch overflow"))?;
                if payload.invocation_epoch != next_epoch {
                    return Err(self.error("retry claim must advance to the next invocation epoch"));
                }
                side_effect_claim_projection(
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.claim_owner,
                    payload.invocation_epoch,
                    payload.claim_generation,
                    &payload.claim_fencing_token,
                )
            }
            _ => return Err(self.error("claim requires intent or not-submitted phase")),
        };
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Claimed { claim: next_claim };
        Ok(self)
    }

    pub(crate) fn take_over(
        mut self,
        event_id: EventId,
        payload: &side_effect::ClaimTakenOver,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_claimed_or_prepared_claim()?;
        if claim.node_id != payload.node_id
            || claim.attempt_id != payload.attempt_id
            || claim.claim_owner != payload.previous_claim_owner
            || claim.claim_generation != payload.previous_claim_generation
            || claim.invocation_epoch != payload.invocation_epoch
        {
            return Err(self.error("takeover claim context does not match active claim"));
        }
        if payload.claim_generation <= payload.previous_claim_generation {
            return Err(self.error("takeover claim generation must increase"));
        }
        if payload.claim_fencing_token == claim.claim_fencing_token {
            return Err(self.error("takeover claim fencing token must change"));
        }
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Claimed {
            claim: side_effect_claim_projection(
                &payload.node_id,
                &payload.attempt_id,
                &payload.new_claim_owner,
                payload.invocation_epoch,
                payload.claim_generation,
                &payload.claim_fencing_token,
            ),
        };
        Ok(self)
    }

    pub(crate) fn prepare(
        mut self,
        event_id: EventId,
        payload: &side_effect::InvocationPrepared,
        prepared_invocation: Option<SideEffectArtifactProjection>,
        resource_key: Option<events::ResourceKeyEvidence>,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_phase_claim("claim")?.clone();
        require_claim_context_for_payload(
            &self.core.ledger_key,
            &claim,
            &payload.node_id,
            &payload.attempt_id,
            payload.invocation_epoch,
            payload.claim_generation,
            &payload.claim_fencing_token,
            None,
        )?;
        self.core.event_id = event_id;
        self.retained.prepared_invocation =
            prepared_invocation.or(self.retained.prepared_invocation);
        self.retained.resource_key = resource_key;
        self.phase = OwnedSideEffectLedgerPhase::Prepared { claim };
        Ok(self)
    }

    pub(crate) fn start(
        mut self,
        event_id: EventId,
        payload: &side_effect::InvocationStarted,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_phase_claim("prepared")?.clone();
        require_claim_context_for_payload(
            &self.core.ledger_key,
            &claim,
            &payload.node_id,
            &payload.attempt_id,
            payload.invocation_epoch,
            payload.claim_generation,
            &payload.claim_fencing_token,
            Some(&payload.claim_owner),
        )?;
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Started { claim };
        Ok(self)
    }

    pub(crate) fn mark_not_submitted(
        self,
        event_id: EventId,
        payload: &side_effect::NotSubmittedProven,
    ) -> Result<Self> {
        self.submission_result(
            SubmissionResultContext {
                event_id,
                ledger_key: &payload.ledger_key,
                ledger_purpose: &payload.ledger_purpose,
                pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
            |claim| OwnedSideEffectLedgerPhase::NotSubmitted {
                claim: claim.clone(),
            },
        )
    }

    pub(crate) fn record_submission(
        mut self,
        event_id: EventId,
        payload: &side_effect::SubmissionObserved,
    ) -> Result<Self> {
        let next = self.submission_result(
            SubmissionResultContext {
                event_id,
                ledger_key: &payload.ledger_key,
                ledger_purpose: &payload.ledger_purpose,
                pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
            |claim| OwnedSideEffectLedgerPhase::SubmissionObserved {
                claim: claim.clone(),
            },
        )?;
        self = next;
        self.retained.submission = Some(SideEffectArtifactProjection {
            artifact_id: payload.submission_artifact_id.clone(),
            content_digest: payload.submission_hash.clone(),
            evidence_hash: payload.submission_artifact_evidence_hash.clone(),
            schema_id: Some(payload.submission_schema_id.clone()),
        });
        Ok(self)
    }

    pub(crate) fn mark_submission_unknown(
        self,
        event_id: EventId,
        payload: &side_effect::SubmissionUnknown,
    ) -> Result<Self> {
        self.submission_result(
            SubmissionResultContext {
                event_id,
                ledger_key: &payload.ledger_key,
                ledger_purpose: &payload.ledger_purpose,
                pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
            |claim| OwnedSideEffectLedgerPhase::SubmissionUnknown {
                claim: claim.clone(),
            },
        )
    }

    pub(crate) fn record_receipt(
        mut self,
        event_id: EventId,
        payload: &side_effect::ReceiptObserved,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_observed_submission_claim()?.clone();
        require_claim_or_verify_context_for_observation(
            &self.core.ledger_key,
            &claim,
            ObservationClaimContext {
                ledger_pair_id: &self.core.pair_id,
                payload_pair_id: &payload.pair_id,
                payload_pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
        )?;
        self.core.event_id = event_id;
        self.retained.receipt = Some(SideEffectArtifactProjection {
            artifact_id: payload.receipt_artifact_id.clone(),
            content_digest: payload.receipt_hash.clone(),
            evidence_hash: payload.receipt_artifact_evidence_hash.clone(),
            schema_id: Some(payload.receipt_schema_id.clone()),
        });
        if let Some(touched_set) = payload.resource_touched_set.clone() {
            self.retained.resource_touched_set = Some(touched_set);
        }
        self.phase = OwnedSideEffectLedgerPhase::ReceiptObserved { claim };
        Ok(self)
    }

    pub(crate) fn confirm(
        mut self,
        event_id: EventId,
        payload: &side_effect::ConfirmationObserved,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_phase_claim("receipt")?.clone();
        require_claim_or_verify_context_for_observation(
            &self.core.ledger_key,
            &claim,
            ObservationClaimContext {
                ledger_pair_id: &self.core.pair_id,
                payload_pair_id: &payload.pair_id,
                payload_pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
        )?;
        self.core.event_id = event_id;
        self.retained.confirmation = Some(SideEffectArtifactProjection {
            artifact_id: payload.confirmation_artifact_id.clone(),
            content_digest: payload.confirmation_hash.clone(),
            evidence_hash: payload.confirmation_artifact_evidence_hash.clone(),
            schema_id: Some(payload.confirmation_schema_id.clone()),
        });
        if let Some(touched_set) = payload.resource_touched_set.clone() {
            self.retained.resource_touched_set = Some(touched_set);
        }
        self.phase = OwnedSideEffectLedgerPhase::Confirmed { claim };
        Ok(self)
    }

    pub(crate) fn mark_ambiguous(
        mut self,
        event_id: EventId,
        payload: &side_effect::Ambiguous,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_ambiguity_source_claim()?.clone();
        require_claim_or_verify_context_for_observation(
            &self.core.ledger_key,
            &claim,
            ObservationClaimContext {
                ledger_pair_id: &self.core.pair_id,
                payload_pair_id: &payload.pair_id,
                payload_pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
        )?;
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Ambiguous {
            claim,
            invocation_epoch: payload.invocation_epoch,
        };
        Ok(self)
    }

    pub(super) fn fail(mut self, event_id: EventId, payload: &side_effect::Failed) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = match payload.failure_phase {
            side_effect::FailurePhase::BeforeInvocationStarted => match &self.phase {
                OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch } => {
                    if payload.invocation_epoch != *invocation_epoch {
                        return Err(self.error("failure invocation epoch does not match intent"));
                    }
                    self.require_intent_context(
                        &payload.node_id,
                        &payload.attempt_id,
                        payload.invocation_epoch,
                    )?;
                    None
                }
                OwnedSideEffectLedgerPhase::Claimed { claim }
                | OwnedSideEffectLedgerPhase::Prepared { claim } => {
                    require_claim_or_verify_context_for_observation(
                        &self.core.ledger_key,
                        claim,
                        ObservationClaimContext {
                            ledger_pair_id: &self.core.pair_id,
                            payload_pair_id: &payload.pair_id,
                            payload_pair_role: payload.pair_role,
                            node_id: &payload.node_id,
                            attempt_id: &payload.attempt_id,
                            invocation_epoch: payload.invocation_epoch,
                        },
                    )?;
                    Some(claim.clone())
                }
                _ => {
                    return Err(self
                        .error("before-start failure requires intent, claim, or prepared phase"));
                }
            },
            side_effect::FailurePhase::AfterNotSubmittedProven => match &self.phase {
                OwnedSideEffectLedgerPhase::NotSubmitted { claim } => {
                    require_claim_or_verify_context_for_observation(
                        &self.core.ledger_key,
                        claim,
                        ObservationClaimContext {
                            ledger_pair_id: &self.core.pair_id,
                            payload_pair_id: &payload.pair_id,
                            payload_pair_role: payload.pair_role,
                            node_id: &payload.node_id,
                            attempt_id: &payload.attempt_id,
                            invocation_epoch: payload.invocation_epoch,
                        },
                    )?;
                    Some(claim.clone())
                }
                _ => {
                    return Err(
                        self.error("after-not-submitted failure requires not-submitted phase")
                    );
                }
            },
        };
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Failed {
            claim,
            invocation_epoch: payload.invocation_epoch,
            failure_phase: payload.failure_phase,
        };
        Ok(self)
    }

    fn submission_result(
        mut self,
        context: SubmissionResultContext<'_>,
        next_phase: impl FnOnce(&SideEffectClaimProjection) -> OwnedSideEffectLedgerPhase,
    ) -> Result<Self> {
        self.require_purpose(context.ledger_key, context.ledger_purpose)?;
        let claim = self
            .require_submission_recovery_claim_for_role(context.pair_role)?
            .clone();
        match context.pair_role {
            events::SideEffectPairRole::Submit => {
                require_claim_identity(
                    &self.core.ledger_key,
                    &claim,
                    context.node_id,
                    context.attempt_id,
                    context.invocation_epoch,
                )?;
            }
            events::SideEffectPairRole::Verify => {
                if claim.invocation_epoch != context.invocation_epoch {
                    return Err(
                        self.error("verify recovery invocation epoch does not match active claim")
                    );
                }
            }
        }
        self.core.event_id = context.event_id;
        self.phase = next_phase(&claim);
        Ok(self)
    }

    fn require_purpose(
        &self,
        ledger_key: &events::SideEffectLedgerKey,
        purpose: &events::SideEffectLedgerPurpose,
    ) -> Result<()> {
        if self.core.ledger_key != *ledger_key {
            return Err(self.error("side-effect ledger key changed"));
        }
        if self.core.ledger_purpose != *purpose {
            return Err(self.error("side-effect ledger purpose changed"));
        }
        Ok(())
    }

    fn require_intent_context(
        &self,
        node_id: &NodeId,
        attempt_id: &AttemptId,
        invocation_epoch: u32,
    ) -> Result<()> {
        self.require_intent_attempt_context(node_id, attempt_id)?;
        if self.core.intent.invocation_epoch != invocation_epoch {
            return Err(self.error("invocation epoch does not match intent projection"));
        }
        Ok(())
    }

    fn require_intent_attempt_context(
        &self,
        node_id: &NodeId,
        attempt_id: &AttemptId,
    ) -> Result<()> {
        if self.core.intent.node_id != *node_id {
            return Err(self.error("node id does not match intent projection"));
        }
        if self.core.intent.attempt_id != *attempt_id {
            return Err(self.error("attempt id does not match intent projection"));
        }
        Ok(())
    }

    fn require_phase_claim(&self, expected: &'static str) -> Result<&SideEffectClaimProjection> {
        match (&self.phase, expected) {
            (OwnedSideEffectLedgerPhase::Claimed { claim }, "claim")
            | (OwnedSideEffectLedgerPhase::Prepared { claim }, "prepared")
            | (OwnedSideEffectLedgerPhase::ReceiptObserved { claim }, "receipt") => Ok(claim),
            _ => Err(self.error(format!(
                "illegal side-effect transition; expected {expected}"
            ))),
        }
    }

    fn require_claimed_or_prepared_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::Claimed { claim }
            | OwnedSideEffectLedgerPhase::Prepared { claim } => Ok(claim),
            _ => Err(self.error("takeover requires claim or prepared phase")),
        }
    }

    fn require_submission_recovery_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::Started { claim }
            | OwnedSideEffectLedgerPhase::SubmissionUnknown { claim } => Ok(claim),
            _ => Err(self.error("submission recovery requires started or unknown phase")),
        }
    }

    fn require_submission_recovery_claim_for_role(
        &self,
        pair_role: events::SideEffectPairRole,
    ) -> Result<&SideEffectClaimProjection> {
        match pair_role {
            events::SideEffectPairRole::Submit => self.require_submission_recovery_claim(),
            events::SideEffectPairRole::Verify => match &self.phase {
                OwnedSideEffectLedgerPhase::SubmissionUnknown { claim } => Ok(claim),
                _ => Err(self.error("verify submission recovery requires unknown phase")),
            },
        }
    }

    fn require_observed_submission_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::SubmissionObserved { claim } => {
                if self.retained.submission.is_none() {
                    return Err(self.error("submission evidence is missing"));
                }
                Ok(claim)
            }
            _ => Err(self.error("receipt requires submission-observed phase")),
        }
    }

    fn require_ambiguity_source_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::Started { claim }
            | OwnedSideEffectLedgerPhase::SubmissionUnknown { claim }
            | OwnedSideEffectLedgerPhase::SubmissionObserved { claim }
            | OwnedSideEffectLedgerPhase::ReceiptObserved { claim } => Ok(claim),
            _ => {
                Err(self.error("ambiguity requires started, unknown, submitted, or receipt phase"))
            }
        }
    }

    fn error(&self, message: impl Into<String>) -> StoreError {
        StoreError::ProjectionConflict {
            key: format!("sidefx_pair:{}", self.core.pair_id),
            message: message.into(),
        }
    }
}

impl OwnedSideEffectLedgerPhase {
    fn into_projection_parts(self) -> (Option<SideEffectClaimProjection>, SideEffectPhase) {
        match self {
            Self::IntentPersisted { invocation_epoch } => {
                (None, SideEffectPhase::IntentPersisted { invocation_epoch })
            }
            Self::Claimed { claim } => (
                Some(claim.clone()),
                SideEffectPhase::Claimed {
                    claim_owner: claim.claim_owner,
                    invocation_epoch: claim.invocation_epoch,
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: claim.claim_fencing_token,
                },
            ),
            Self::Prepared { claim } => (
                Some(claim.clone()),
                SideEffectPhase::InvocationPrepared {
                    invocation_epoch: claim.invocation_epoch,
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: claim.claim_fencing_token,
                },
            ),
            Self::Started { claim } => (
                Some(claim.clone()),
                SideEffectPhase::InvocationStarted {
                    claim_owner: claim.claim_owner,
                    invocation_epoch: claim.invocation_epoch,
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: claim.claim_fencing_token,
                },
            ),
            Self::SubmissionObserved { claim } => (
                Some(claim.clone()),
                SideEffectPhase::SubmissionObserved {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::NotSubmitted { claim } => (
                Some(claim.clone()),
                SideEffectPhase::NotSubmittedProven {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::SubmissionUnknown { claim } => (
                Some(claim.clone()),
                SideEffectPhase::SubmissionUnknown {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::ReceiptObserved { claim } => (
                Some(claim.clone()),
                SideEffectPhase::ReceiptObserved {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::Confirmed { claim } => (
                Some(claim.clone()),
                SideEffectPhase::ConfirmationObserved {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::Ambiguous {
                claim,
                invocation_epoch,
            } => (Some(claim), SideEffectPhase::Ambiguous { invocation_epoch }),
            Self::Failed {
                claim,
                invocation_epoch,
                failure_phase,
            } => (
                claim,
                SideEffectPhase::Failed {
                    invocation_epoch,
                    failure_phase,
                },
            ),
        }
    }
}
