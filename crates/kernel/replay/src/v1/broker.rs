use super::*;

#[path = "broker/artifacts.rs"]
mod artifacts;
#[path = "broker/facts.rs"]
mod facts;
#[path = "broker/output.rs"]
mod output;
#[path = "broker/queries.rs"]
mod queries;
#[path = "broker/side_effects.rs"]
mod side_effects;

impl ReplayBroker {
    /// Builds a replay broker from sealed replay read authority.
    pub fn from_read_authority(authority: ReplayReadAuthority) -> Result<Self> {
        Self::from_validated_parts(authority)
    }

    fn from_validated_parts(authority: ReplayReadAuthority) -> Result<Self> {
        let certified_spec = authority.certified_spec.clone();
        let stream = authority.stream.clone();
        certified_spec.verify_hash()?;
        ProjectionSnapshot::validate_run_stream(&stream)?;
        let retained_artifacts = artifact_map(authority.artifact_evidence.clone())?;
        let artifact_byte_authority =
            artifact_byte_authority_map(&retained_artifacts, &authority.artifact_bytes)?;
        let projection = ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
            &stream,
            &artifact_byte_authority,
        )?;
        let run_admitted = run_admitted_payload(&stream)?;

        if run_admitted.spec_hash != certified_spec.spec_hash {
            return Err(ReplayError::new(
                ReplayErrorKind::SpecHashMismatch,
                "run-start spec hash does not match certified spec",
            ));
        }
        if stream
            .iter()
            .any(|event| event.spec_hash() != &certified_spec.spec_hash)
        {
            return Err(ReplayError::new(
                ReplayErrorKind::SpecHashMismatch,
                "run stream contains payloads for a different certified spec",
            ));
        }
        verify_run_start_contract(
            &certified_spec,
            &run_admitted,
            &authority,
            &retained_artifacts,
        )?;
        verify_remediation_ledger_links(&certified_spec, &projection)?;
        verify_resource_lane_release_adjacency(&certified_spec, &stream)?;

        let mut broker = Self {
            certified_spec,
            stream: stream.clone(),
            run_id: run_admitted,
            projection,
            retained_artifacts,
            artifact_bytes: authority.artifact_bytes.clone(),
            artifact_byte_authority,
            artifacts: BTreeMap::new(),
            facts: BTreeMap::new(),
            fact_events: BTreeMap::new(),
            intents: BTreeMap::new(),
            prepared_invocations: BTreeMap::new(),
            submissions: BTreeMap::new(),
            submission_unknown: BTreeMap::new(),
            not_submitted: BTreeMap::new(),
            receipts: BTreeMap::new(),
            confirmations: BTreeMap::new(),
            ambiguities: BTreeMap::new(),
            manual_resolutions: BTreeMap::new(),
        };
        broker.authorize_certified_spec_artifacts()?;
        broker.authorize_additional_artifacts(&authority.additional_artifact_evidence)?;
        broker.index_retained_source_fact_events(&authority.source_fact_events)?;
        broker.index_stream(&stream)?;
        broker.verify_terminal_outcome_agreement()?;
        broker.reject_unauthorized_artifact_evidence()?;
        Ok(broker)
    }

    fn authorize_certified_spec_artifacts(&mut self) -> Result<()> {
        let config_refs = self.certified_spec.spec.config_refs.clone();
        for config in config_refs {
            let config_evidence = store::ArtifactEvidenceRef {
                artifact_id: config.artifact_id.clone(),
                digest: config.digest.clone(),
                byte_len: config.byte_len,
                media_type: config.media_type.clone(),
                schema_id: Some(config.schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: ArtifactRole::TypedConfig,
            };
            let evidence_hash = config_evidence.evidence_hash().map_err(ReplayError::from)?;
            self.authorize_artifact(ArtifactEvidenceExpectation {
                artifact_id: &config.artifact_id,
                evidence_hash: &evidence_hash,
                digest: &config.digest,
                schema_id: Some(&config.schema_id),
                semantic_type_id: None,
                role: ArtifactRole::TypedConfig,
                producer_node_id: None,
                producer_seed_id: None,
            })?;
        }
        Ok(())
    }

    fn authorize_additional_artifacts(
        &mut self,
        artifacts: &[StoredArtifactEvidenceRef],
    ) -> Result<()> {
        for artifact in artifacts {
            self.insert_authorized_artifact(artifact.clone())?;
        }
        Ok(())
    }

    fn index_stream(&mut self, stream: &[KernelEventEnvelope]) -> Result<()> {
        let mut resource_keys = BTreeMap::new();
        for envelope in stream {
            match envelope.payload() {
                KernelEventPayload::RunAdmitted(payload) => {
                    for seed in &payload.seed_cells {
                        self.verify_seed_against_spec(seed)?;
                    }
                    self.authorize_event_artifacts(envelope.payload())?;
                }
                KernelEventPayload::StateAttemptStarted(payload) => {
                    self.verify_state_attempt_started_against_spec(payload)?;
                }
                KernelEventPayload::FactRecorded(payload) => {
                    self.verify_fact_against_spec(payload)?;
                    self.authorize_event_artifacts(envelope.payload())?;
                    self.insert_fact_event(envelope, payload)?;
                }
                KernelEventPayload::ArtifactReferenced(payload) => {
                    if let Some(node_id) = &payload.node_id {
                        self.node(node_id)?;
                    }
                    self.authorize_event_artifacts(envelope.payload())?;
                    if payload.artifact_ref.role == ArtifactRole::FactQueryEvidence {
                        self.verify_fact_query_evidence_reference(payload)?;
                    }
                }
                KernelEventPayload::SideEffectIntentPersisted(payload) => {
                    self.verify_side_effect_intent_against_spec(payload)?;
                    self.authorize_event_artifacts(envelope.payload())?;
                    insert_unique(
                        &mut self.intents,
                        payload.pair_id.clone(),
                        payload.clone(),
                        ReplayErrorKind::InvalidRunStream,
                        "duplicate side-effect intent replay event",
                    )?;
                }
                KernelEventPayload::SideEffectClaimed(payload) => {
                    self.verify_side_effect_event_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                KernelEventPayload::SideEffectClaimTakenOver(payload) => {
                    self.verify_side_effect_event_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                KernelEventPayload::ResourceLaneClaimed(payload) => {
                    self.verify_resource_lane_claim(payload, &mut resource_keys)?;
                }
                KernelEventPayload::ResourceLaneReleased(payload) => {
                    self.verify_resource_lane_release_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        payload.release_authority,
                    )?;
                }
                KernelEventPayload::ResourceLaneClaimIntent(_)
                | KernelEventPayload::ResourceLaneReleaseIntent(_) => {
                    return Err(ReplayError::new(
                        ReplayErrorKind::InvalidRunStream,
                        "replay stream contains unmaterialized resource-lane intent",
                    ));
                }
                KernelEventPayload::SideEffectInvocationStarted(payload) => {
                    self.verify_side_effect_event_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                    self.verify_side_effect_event_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                    self.authorize_event_artifacts(envelope.payload())?;
                    insert_unique(
                        &mut self.submissions,
                        (payload.pair_id.clone(), payload.invocation_epoch),
                        payload.clone(),
                        ReplayErrorKind::InvalidRunStream,
                        "duplicate side-effect submission replay event",
                    )?;
                }
                KernelEventPayload::SideEffectReceiptObserved(payload) => {
                    self.verify_side_effect_event_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                    self.verify_resource_touched_set(
                        &payload.pair_id,
                        payload.pair_role,
                        &payload.node_id,
                        payload.resource_touched_set.as_ref(),
                    )?;
                    self.authorize_event_artifacts(envelope.payload())?;
                    insert_unique(
                        &mut self.receipts,
                        (payload.pair_id.clone(), payload.invocation_epoch),
                        payload.clone(),
                        ReplayErrorKind::InvalidRunStream,
                        "duplicate side-effect receipt replay event",
                    )?;
                }
                KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                    self.verify_side_effect_event_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                    self.verify_resource_touched_set(
                        &payload.pair_id,
                        payload.pair_role,
                        &payload.node_id,
                        payload.resource_touched_set.as_ref(),
                    )?;
                    self.authorize_event_artifacts(envelope.payload())?;
                    insert_unique(
                        &mut self.confirmations,
                        (payload.pair_id.clone(), payload.invocation_epoch),
                        payload.clone(),
                        ReplayErrorKind::InvalidRunStream,
                        "duplicate side-effect confirmation replay event",
                    )?;
                }
                KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                    self.verify_side_effect_event_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                    self.verify_invocation_prepared_resource_key(payload, &resource_keys)?;
                    self.authorize_event_artifacts(envelope.payload())?;
                    insert_unique(
                        &mut self.prepared_invocations,
                        (payload.pair_id.clone(), payload.invocation_epoch),
                        payload.clone(),
                        ReplayErrorKind::InvalidRunStream,
                        "duplicate side-effect prepared invocation replay event",
                    )?;
                }
                KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                    self.verify_side_effect_event_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                    self.authorize_event_artifacts(envelope.payload())?;
                    insert_unique(
                        &mut self.not_submitted,
                        (payload.pair_id.clone(), payload.invocation_epoch),
                        payload.clone(),
                        ReplayErrorKind::InvalidRunStream,
                        "duplicate side-effect not-submitted replay event",
                    )?;
                }
                KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                    self.verify_side_effect_event_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                    self.authorize_event_artifacts(envelope.payload())?;
                    insert_unique(
                        &mut self.submission_unknown,
                        (payload.pair_id.clone(), payload.invocation_epoch),
                        payload.clone(),
                        ReplayErrorKind::InvalidRunStream,
                        "duplicate side-effect submission-unknown replay event",
                    )?;
                }
                KernelEventPayload::SideEffectAmbiguous(payload) => {
                    self.verify_side_effect_ambiguity_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                    self.authorize_event_artifacts(envelope.payload())?;
                    insert_unique(
                        &mut self.ambiguities,
                        (payload.pair_id.clone(), payload.invocation_epoch),
                        payload.clone(),
                        ReplayErrorKind::InvalidRunStream,
                        "duplicate side-effect ambiguity replay event",
                    )?;
                }
                KernelEventPayload::CellProduced(payload) => {
                    self.verify_cell_produced_against_spec(payload)?;
                    self.authorize_event_artifacts(envelope.payload())?;
                }
                KernelEventPayload::CellSkipped(payload) => {
                    self.verify_cell_skipped_against_spec(payload)?;
                }
                KernelEventPayload::PublicOutputProduced(payload) => {
                    self.verify_public_output_against_spec(payload)?;
                    self.authorize_event_artifacts(envelope.payload())?;
                }
                KernelEventPayload::PublicOutputRenderFailed(payload) => {
                    self.verify_public_output_render_failure_against_spec(payload)?;
                    self.authorize_event_artifacts(envelope.payload())?;
                }
                KernelEventPayload::StateAttemptFailed(payload) => {
                    self.node(&payload.node_id)?;
                    self.authorize_event_artifacts(envelope.payload())?;
                }
                KernelEventPayload::StateAttemptInterrupted(payload) => {
                    self.node(&payload.node_id)?;
                }
                KernelEventPayload::StateAttemptCompleted(payload) => {
                    self.verify_state_attempt_completed_against_spec(payload)?;
                }
                KernelEventPayload::RunCompleted(payload) => match &payload.outcome {
                    events::RunCompletionOutcome::Completed(evidence) => {
                        self.verify_completed_run_public_output(evidence)?;
                    }
                    events::RunCompletionOutcome::Compensated
                    | events::RunCompletionOutcome::ManuallyResolved
                    | events::RunCompletionOutcome::FailedWithoutAcdcClaim => {}
                },
                KernelEventPayload::ManualResolutionRecorded(payload) => {
                    let verified = self.verify_manual_resolution_against_spec(envelope, payload)?;
                    insert_unique(
                        &mut self.manual_resolutions,
                        payload.run_id.clone(),
                        verified,
                        ReplayErrorKind::InvalidRunStream,
                        "duplicate manual resolution replay event",
                    )?;
                    self.authorize_event_artifacts(envelope.payload())?;
                }
                KernelEventPayload::RetentionManifestProjected(_) => {
                    self.authorize_event_artifacts(envelope.payload())?;
                }
                KernelEventPayload::RetentionRefsAppended(_) => {
                    self.authorize_event_artifacts(envelope.payload())?;
                }
                KernelEventPayload::SideEffectFailed(payload) => {
                    self.verify_side_effect_event_against_intent(
                        &payload.pair_id,
                        &payload.ledger_key,
                        payload.invocation_epoch,
                        payload.pair_role,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn index_retained_source_fact_events(
        &mut self,
        source_fact_events: &[RetainedSourceFactReplayEvent],
    ) -> Result<()> {
        // Cross-run source facts are external authority for FactQueryEvidence returned-refs.
        // They may originate from other certified programs (e.g. collectors → report), so
        // they are NOT re-validated against this consumer program's node graph or
        // certified_spec_hash. Claim content is bound by verify_fact_query_returned_ref
        // against the authenticated InternalFactRef in retained query evidence.
        for source in source_fact_events {
            let envelope = source.envelope();
            let KernelEventPayload::FactRecorded(payload) = envelope.payload() else {
                return Err(ReplayError::new(
                    ReplayErrorKind::FactMismatch,
                    "retained source fact event payload is not FactRecorded",
                ));
            };
            self.authorize_event_artifacts(envelope.payload())?;
            self.insert_fact_event(envelope, payload)?;
        }
        Ok(())
    }

    fn insert_fact_event(
        &mut self,
        envelope: &KernelEventEnvelope,
        payload: &events::FactRecorded,
    ) -> Result<()> {
        let fact_claim_id = mfm_facts::derive_fact_claim_id(
            envelope.run_id().clone(),
            envelope.seq().as_u64(),
            envelope.ordinal().as_u32(),
        )
        .map_err(|error| ReplayError::new(ReplayErrorKind::InvalidRunStream, error.to_string()))?;
        match (
            self.facts.get(&fact_claim_id),
            self.fact_events.get(&fact_claim_id),
        ) {
            (Some(existing_payload), Some(existing_envelope))
                if existing_payload == payload && existing_envelope == envelope =>
            {
                return Ok(());
            }
            (Some(_), _) | (_, Some(_)) => {
                return Err(ReplayError::new(
                    ReplayErrorKind::InvalidRunStream,
                    "duplicate fact replay event",
                ));
            }
            (None, None) => {}
        }
        self.facts.insert(fact_claim_id.clone(), payload.clone());
        self.fact_events.insert(fact_claim_id, envelope.clone());
        Ok(())
    }

    fn verify_node_capability(
        &self,
        node_id: &NodeId,
        capability_kind: &CapabilityKind,
        capability_version: &CapabilityVersion,
    ) -> Result<()> {
        let node = self.node(node_id)?;
        if capability_set_contains(
            &node.capability_bindings,
            capability_kind,
            capability_version,
        ) {
            Ok(())
        } else {
            Err(ReplayError::new(
                ReplayErrorKind::UnsupportedCapability,
                format!("node {node_id} is not certified for capability {capability_kind}"),
            ))
        }
    }

    fn verify_node_adapter(
        &self,
        node_id: &NodeId,
        adapter_kind: &AdapterKind,
        adapter_version: &AdapterVersion,
    ) -> Result<()> {
        let node = self.node(node_id)?;
        if node.adapter_bindings.iter().any(|binding| {
            binding.adapter_kind == *adapter_kind && binding.adapter_version == *adapter_version
        }) {
            Ok(())
        } else {
            Err(ReplayError::new(
                ReplayErrorKind::UnsupportedAdapter,
                format!("node {node_id} is not certified for adapter {adapter_kind}"),
            ))
        }
    }

    fn node(&self, node_id: &NodeId) -> Result<&spec::NodeSpec> {
        self.certified_spec
            .spec
            .nodes
            .iter()
            .chain(self.certified_spec.spec.remediations.values())
            .find(|node| &node.node_id == node_id)
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::CertifiedSpec,
                    format!("node {node_id} is not present in certified spec"),
                )
            })
    }

    fn cell(&self, cell_id: &mfm_ids::CellId) -> Result<&spec::CellSpec> {
        self.certified_spec
            .spec
            .cells
            .iter()
            .find(|cell| &cell.cell_id == cell_id)
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::CertifiedEvidenceMismatch,
                    format!("cell {cell_id} is not present in certified spec"),
                )
            })
    }

    fn is_terminal_lifecycle_receipt_artifact(
        &self,
        node_id: &NodeId,
        artifact_id: &ArtifactId,
        digest: &ContentDigest,
        evidence_hash: &ContentDigest,
    ) -> Result<bool> {
        let node = self.node(node_id)?;
        if !is_terminal_lifecycle_node(node) {
            return Ok(false);
        }
        Ok(matches!(
            self.projection
                .cell_terminal_for_run(&self.run_id.run_id, &node.output_cell),
            Some(store::CellTerminalProjection::Produced {
                artifact_id: projected_artifact_id,
                content_digest,
                evidence_hash: projected_evidence_hash,
                ..
            }) if projected_artifact_id == artifact_id
                && content_digest == digest
                && projected_evidence_hash == evidence_hash
        ))
    }

    fn verify_seed_against_spec(&self, seed: &events::SeedCellRef) -> Result<()> {
        let certified_seed = self
            .certified_spec
            .spec
            .seeds
            .iter()
            .find(|certified_seed| certified_seed.seed_id == seed.seed_id)
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::CertifiedEvidenceMismatch,
                    format!("seed {} is not present in certified spec", seed.seed_id),
                )
            })?;
        if certified_seed.cell_id != seed.cell_id
            || certified_seed.scope_id != seed.scope_id
            || certified_seed.semantic_type_id != seed.semantic_type_id
            || certified_seed.schema_id != seed.schema_id
            || certified_seed
                .required_digest
                .as_ref()
                .is_some_and(|digest| digest != &seed.digest)
        {
            return Err(certified_evidence_mismatch(
                "run-start seed cell does not match certified spec",
            ));
        }
        let cell = self.cell(&seed.cell_id)?;
        if cell.producer != spec::CellProducer::Seed(seed.seed_id.clone())
            || cell.scope_id != seed.scope_id
            || cell.semantic_type_id != seed.semantic_type_id
            || cell.schema_id != seed.schema_id
        {
            return Err(certified_evidence_mismatch(
                "run-start seed cell terminal does not match certified spec",
            ));
        }
        Ok(())
    }

    fn verify_state_attempt_started_against_spec(
        &self,
        payload: &events::StateAttemptStarted,
    ) -> Result<()> {
        let node = self.node(&payload.node_id)?;
        if node.state_kind != payload.state_kind || node.state_version != payload.state_version {
            return Err(certified_evidence_mismatch(
                "state attempt start does not match certified node identity",
            ));
        }
        Ok(())
    }

    fn verify_state_attempt_completed_against_spec(
        &self,
        payload: &events::StateAttemptCompleted,
    ) -> Result<()> {
        let node = self.node(&payload.node_id)?;
        if node.output_cell != payload.output_cell_id {
            return Err(certified_evidence_mismatch(
                "state attempt completion output cell does not match certified node",
            ));
        }
        Ok(())
    }

    fn verify_fact_against_spec(&self, payload: &events::FactRecorded) -> Result<()> {
        let node = self.node(&payload.node_id)?;
        if !node
            .fact_descriptor_allowlist
            .iter()
            .any(|reference| &reference.descriptor_hash == payload.claim.fact_descriptor_hash())
        {
            return Err(ReplayError::new(
                ReplayErrorKind::CertifiedEvidenceMismatch,
                format!(
                    "fact descriptor {} is not certified for producing node {}",
                    payload.claim.fact_descriptor_hash(),
                    payload.node_id
                ),
            ));
        }
        let producer = payload.claim.producer();
        self.verify_node_capability(
            &payload.node_id,
            producer.capability_kind(),
            producer.capability_version(),
        )?;
        self.verify_node_adapter(
            &payload.node_id,
            producer.adapter_kind(),
            producer.adapter_version(),
        )?;
        Ok(())
    }

    fn verify_side_effect_intent_against_spec(
        &self,
        payload: &side_effect::IntentPersisted,
    ) -> Result<()> {
        let node = self.node(&payload.node_id)?;
        if node.scope_id != payload.scope_id || node.side_effect.is_none() {
            return Err(certified_evidence_mismatch(
                "side-effect intent does not match certified side-effect node",
            ));
        }
        let contract =
            CertifiedSideEffectContract::for_node(&self.certified_spec.spec, &payload.node_id)
                .map_err(certified_contract_mismatch)?;
        let forward = match &payload.ledger_purpose {
            events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
                let forward = self
                    .projection
                    .side_effect_for_pair(&self.run_id.run_id, forward_pair_id);
                forward
            }
            events::SideEffectLedgerPurpose::Forward => None,
        };
        let terminal_policies =
            store::SideEffectTerminalPolicies::from_spec(&self.certified_spec.spec)
                .map_err(store_error)?;
        contract
            .validate_remediation_link(CertifiedRemediationLink {
                remediation_run_id: &self.run_id.run_id,
                ledger_purpose: &payload.ledger_purpose,
                forward_run_id: forward.map(|projection| &projection.run_id),
                forward_node_id: forward.map(|projection| &projection.intent.node_id),
                forward_ledger_purpose: forward.map(|projection| &projection.ledger_purpose),
                forward_terminal: forward
                    .map(|projection| {
                        terminal_policies
                            .require(&projection.pair_id)
                            .map(|policy| policy.is_terminal_phase(&projection.phase))
                    })
                    .transpose()
                    .map_err(store_error)?
                    .unwrap_or(false),
            })
            .map_err(certified_contract_mismatch)?;
        self.verify_node_capability(
            &payload.node_id,
            &payload.capability_kind,
            &payload.capability_version,
        )?;
        self.verify_node_adapter(
            &payload.node_id,
            &payload.adapter_kind,
            &payload.adapter_version,
        )?;
        Ok(())
    }

    fn verify_invocation_prepared_resource_key(
        &self,
        payload: &side_effect::InvocationPrepared,
        resource_keys: &BTreeMap<SideEffectPairId, events::ResourceKeyEvidence>,
    ) -> Result<()> {
        self.node(&payload.node_id)?;
        let contract =
            CertifiedSideEffectContract::for_node(&self.certified_spec.spec, &payload.node_id)
                .map_err(certified_contract_mismatch)?;
        contract
            .validate_epoch_resource_consistency(
                resource_keys.get(&payload.pair_id),
                payload.resource_key.as_ref(),
            )
            .map_err(certified_contract_mismatch)?;
        Ok(())
    }

    fn verify_resource_lane_claim(
        &self,
        payload: &events::ResourceLaneClaimed,
        resource_keys: &mut BTreeMap<SideEffectPairId, events::ResourceKeyEvidence>,
    ) -> Result<()> {
        self.verify_side_effect_event_against_intent(
            &payload.pair_id,
            &payload.ledger_key,
            payload.invocation_epoch,
            payload.pair_role,
            &payload.node_id,
            &payload.attempt_id,
        )?;
        let contract =
            CertifiedSideEffectContract::for_node(&self.certified_spec.spec, &payload.node_id)
                .map_err(certified_contract_mismatch)?;
        contract
            .validate_epoch_resource_consistency(
                resource_keys.get(&payload.pair_id),
                Some(&payload.resource_key),
            )
            .map_err(certified_contract_mismatch)?;
        resource_keys.insert(payload.pair_id.clone(), payload.resource_key.clone());
        Ok(())
    }

    fn verify_resource_lane_release_against_intent(
        &self,
        pair_id: &SideEffectPairId,
        ledger_key: &events::SideEffectLedgerKey,
        invocation_epoch: u32,
        pair_role: events::SideEffectPairRole,
        release_authority: events::ResourceLaneReleaseAuthority,
    ) -> Result<()> {
        if pair_role != events::SideEffectPairRole::Verify {
            return Err(side_effect_mismatch(
                "resource lane release requires verify pair role",
            ));
        }
        match release_authority {
            events::ResourceLaneReleaseAuthority::VerifyTerminal
            | events::ResourceLaneReleaseAuthority::ManualResolution => {}
        }
        let intent = self.side_effect_intent_for_event(pair_id, ledger_key, invocation_epoch)?;
        let (_verify_node, verify) = self.side_effect_verify_node_for_pair(pair_id)?;
        if verify.submit_node_id != intent.node_id {
            return Err(side_effect_mismatch(
                "resource lane release does not match certified pair",
            ));
        }
        Ok(())
    }

    fn verify_resource_touched_set(
        &self,
        pair_id: &SideEffectPairId,
        pair_role: events::SideEffectPairRole,
        node_id: &NodeId,
        touched_set: Option<&events::ResourceTouchedSetEvidence>,
    ) -> Result<()> {
        let contract_node_id =
            self.side_effect_contract_node_for_pair_event(pair_id, pair_role, node_id)?;
        CertifiedSideEffectContract::for_node(&self.certified_spec.spec, contract_node_id)
            .and_then(|contract| contract.validate_touched_set(touched_set))
            .map_err(certified_contract_mismatch)
    }

    fn verify_manual_resolution_against_spec(
        &self,
        envelope: &KernelEventEnvelope,
        payload: &events::ManualResolutionRecorded,
    ) -> Result<VerifiedManualResolutionForPrefix> {
        let manual =
            certified_manual_resolution_spec(&self.certified_spec.spec.saga).ok_or_else(|| {
                certified_evidence_mismatch(
                    "manual resolution was recorded without certified manual policy",
                )
            })?;
        if payload.evidence_schema_id != manual.evidence_schema {
            return Err(certified_evidence_mismatch(
                "manual resolution evidence schema does not match certified policy",
            ));
        }
        let authorization_schema_id = manual_authorization_proof_schema_id().map_err(|error| {
            ReplayError::new(
                ReplayErrorKind::CertifiedEvidenceMismatch,
                error.to_string(),
            )
        })?;
        if payload.authorization_schema_id != authorization_schema_id {
            return Err(certified_evidence_mismatch(
                "manual resolution authorization schema does not match certified policy",
            ));
        }
        let manual_start = self
            .stream
            .iter()
            .position(|event| {
                event.seq() == envelope.seq()
                    && event.ordinal() == envelope.ordinal()
                    && event.event_id() == envelope.event_id()
            })
            .ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::InvalidRunStream,
                    "manual resolution payload was not found in replay stream",
                )
            })?;
        let prefix_projection = ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
            &self.stream[..manual_start],
            &self.artifact_byte_authority,
        )
        .map_err(|error| ReplayError::new(ReplayErrorKind::InvalidRunStream, error.to_string()))?;
        let terminal_policies =
            store::SideEffectTerminalPolicies::from_spec(&self.certified_spec.spec)
                .map_err(store_error)?;
        prefix_projection
            .require_manual_resolution_admissible(
                &payload.run_id,
                &self.certified_spec.spec.saga,
                &terminal_policies,
            )
            .map_err(|error| {
                ReplayError::new(
                    ReplayErrorKind::CertifiedEvidenceMismatch,
                    error.to_string(),
                )
            })?;
        let prefix_saga = prefix_projection
            .derive_saga_projection(
                &payload.run_id,
                &self.certified_spec.spec.saga,
                &terminal_policies,
            )
            .map_err(|error| {
                ReplayError::new(
                    ReplayErrorKind::CertifiedEvidenceMismatch,
                    error.to_string(),
                )
            })?;
        let block_reason = prefix_saga.manual_block_reason.ok_or_else(|| {
            certified_evidence_mismatch("manual resolution prefix lacks block reason")
        })?;
        let evidence_requirement = store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::ManualResolutionEvidence,
            artifact_id: payload.evidence_artifact_id.clone(),
            evidence_hash: payload.evidence_artifact_evidence_hash.clone(),
            digest: Some(payload.evidence_hash.clone()),
            byte_len: None,
            media_type: None,
            schema_id: Some(payload.evidence_schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: Some(ArtifactRole::ManualResolutionEvidence),
        };
        let _evidence_artifact = exact_retained_artifact_for_requirement(
            &self.retained_artifacts,
            &evidence_requirement,
        )?;
        let authorization_requirement = store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::ManualResolutionAuthorization,
            artifact_id: payload.authorization_artifact_id.clone(),
            evidence_hash: payload.authorization_artifact_evidence_hash.clone(),
            digest: Some(payload.authorization_hash.clone()),
            byte_len: None,
            media_type: None,
            schema_id: Some(payload.authorization_schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: Some(ArtifactRole::ManualResolutionAuthorization),
        };
        let authorization_artifact = exact_retained_artifact_for_requirement(
            &self.retained_artifacts,
            &authorization_requirement,
        )?;
        let proof_bytes = self.artifact_bytes_for_evidence(authorization_artifact)?;
        let prefix = ManualResolutionPrefixAuthority::new(
            payload.run_id.clone(),
            payload.spec_hash.clone(),
            envelope.seq().as_u64(),
            mfm_runtime::manual_resolution_stream_prefix_digest(&self.stream[..manual_start])?,
            mfm_runtime::manual_resolution_block_reason(block_reason),
            mfm_runtime::unresolved_manual_obligations_digest(&prefix_saga)?,
            manual.clone(),
        )
        .map_err(|error| {
            ReplayError::new(
                ReplayErrorKind::CertifiedEvidenceMismatch,
                format!("manual authorization prefix failed validation: {error}"),
            )
        })?;
        let verified = ManualResolutionProofAuthority::new(
            prefix,
            payload.outcome,
            ManualResolutionEvidenceRef {
                schema_id: payload.evidence_schema_id.clone(),
                content_hash: payload.evidence_hash.clone(),
                artifact_id: payload.evidence_artifact_id.clone(),
            },
            ManualResolutionEvidenceRef {
                schema_id: payload.authorization_schema_id.clone(),
                content_hash: payload.authorization_hash.clone(),
                artifact_id: payload.authorization_artifact_id.clone(),
            },
            proof_bytes.to_vec(),
        )
        .and_then(ManualResolutionProofAuthority::verify)
        .map_err(|error| {
            ReplayError::new(
                ReplayErrorKind::CertifiedEvidenceMismatch,
                format!("manual authorization proof failed verification: {error}"),
            )
        })?;
        Ok(verified)
    }

    fn verify_terminal_outcome_agreement(&self) -> Result<()> {
        let Some((terminal_start, payload)) = terminal_completion_event(&self.stream)? else {
            return Ok(());
        };
        match &payload.outcome {
            events::RunCompletionOutcome::Completed(evidence) => {
                match self
                    .projection
                    .public_output(&self.run_id.run_id, &evidence.public_output_schema_id)
                {
                    Some(store::PublicOutputProjection::Produced { event_id, .. })
                        if event_id == &evidence.public_output_event_id =>
                    {
                        Ok(())
                    }
                    _ => Err(certified_evidence_mismatch(
                        "completed terminal outcome does not match projected public output",
                    )),
                }
            }
            events::RunCompletionOutcome::Compensated
            | events::RunCompletionOutcome::ManuallyResolved
            | events::RunCompletionOutcome::FailedWithoutAcdcClaim => {
                let prefix_next_seq = self.stream[..terminal_start]
                    .last()
                    .map(|event| {
                        let next = event
                            .seq()
                            .as_u64()
                            .checked_add(1)
                            .ok_or(store::StoreError::SequenceOverflow)?;
                        store::StreamSeq::new(next)
                    })
                    .transpose()?
                    .unwrap_or(store::StreamSeq::FIRST);
                let prefix_projection =
                    ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
                        &self.stream[..terminal_start],
                        &self.artifact_byte_authority,
                    )?;
                let terminal_policies =
                    store::SideEffectTerminalPolicies::from_spec(&self.certified_spec.spec)
                        .map_err(store_error)?;
                let saga = prefix_projection.derive_saga_projection(
                    &self.run_id.run_id,
                    &self.certified_spec.spec.saga,
                    &terminal_policies,
                )?;
                let proof = store::SagaTerminalProof::new(
                    &self.certified_spec.spec.saga,
                    &saga,
                    prefix_next_seq,
                    self.manual_resolutions.get(&self.run_id.run_id).cloned(),
                )
                .map_err(|error| {
                    ReplayError::new(
                        ReplayErrorKind::CertifiedEvidenceMismatch,
                        error.to_string(),
                    )
                })?;
                if proof.outcome() == payload.outcome {
                    Ok(())
                } else {
                    Err(ReplayError::new(
                        ReplayErrorKind::CertifiedEvidenceMismatch,
                        "saga terminal outcome does not match proof",
                    ))
                }
            }
        }
    }

    fn reject_unauthorized_artifact_evidence(&self) -> Result<()> {
        for (key, evidence) in &self.retained_artifacts {
            if !self.artifacts.contains_key(key) {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!(
                        "retained artifact evidence supplied without replay authorization for {}",
                        evidence.artifact_id
                    ),
                ));
            }
        }
        for key in self.artifact_bytes.keys() {
            if !self.retained_artifacts.contains_key(key) {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!(
                        "artifact bytes supplied without verified evidence for {}",
                        key.0
                    ),
                ));
            }
        }
        Ok(())
    }

    fn verify_cell_produced_against_spec(&self, payload: &events::CellProduced) -> Result<()> {
        let node = self.node(&payload.node_id)?;
        if node.output_cell != payload.cell_id {
            return Err(certified_evidence_mismatch(
                "produced cell does not match certified node output",
            ));
        }
        let cell = self.cell(&payload.cell_id)?;
        if cell.producer != spec::CellProducer::Node(payload.node_id.clone())
            || cell.scope_id != payload.scope_id
            || cell.semantic_type_id != payload.semantic_type_id
            || cell.schema_id != payload.schema_id
            || cell.value_lineage != payload.value_lineage
            || cell.context != payload.context
        {
            return Err(certified_evidence_mismatch(
                "produced cell does not match certified cell spec",
            ));
        }
        Ok(())
    }

    fn verify_cell_skipped_against_spec(&self, payload: &events::CellSkipped) -> Result<()> {
        let node = self.node(&payload.node_id)?;
        if node.output_cell != payload.cell_id {
            return Err(certified_evidence_mismatch(
                "skipped cell does not match certified node output",
            ));
        }
        let cell = self.cell(&payload.cell_id)?;
        if cell.producer != spec::CellProducer::Node(payload.node_id.clone())
            || cell.scope_id != payload.scope_id
            || cell.semantic_type_id != payload.semantic_type_id
            || cell.schema_id != payload.schema_id
            || cell.value_lineage != payload.value_lineage
            || cell.context != payload.context
        {
            return Err(certified_evidence_mismatch(
                "skipped cell does not match certified cell spec",
            ));
        }
        Ok(())
    }
}
