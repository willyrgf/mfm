use super::*;

#[path = "projection_apply.rs"]
mod projection_apply;
pub(crate) use self::projection_apply::{
    apply_projection, apply_projection_for_external_fact_queries, fact_claim_projection_key,
};

impl ProjectionSnapshot {
    /// Returns a snapshot with store-owned fact authority and resource lanes copied together.
    pub fn with_store_authority_from(&self, authority: &ProjectionSnapshot) -> Result<Self> {
        let mut parts = ProjectionSnapshotParts::from_snapshot(self);
        parts.replace_fact_authority_from(authority);
        parts.replace_resource_lanes_from(authority);
        Self::from_parts(parts)
    }

    /// Creates a projection snapshot from storage-owned projection maps.
    ///
    /// Callers populate only the projection families they hydrate and leave the rest empty via
    /// [`ProjectionSnapshotParts`]'s [`Default`].
    pub fn from_parts(parts: ProjectionSnapshotParts) -> Result<Self> {
        let ProjectionSnapshotParts {
            run_states,
            run_spec_hashes,
            saga_policy_digests,
            run_completions,
            saga_engagements,
            manual_resolutions,
            attempts,
            cells,
            fact_descriptors,
            fact_query_entries,
            fact_term_entries,
            side_effects,
            resource_lanes,
            public_outputs,
            retentions,
        } = parts;
        for ((node_id, attempt_id), projection) in &attempts {
            if node_id != &projection.node_id || attempt_id != &projection.attempt_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!("attempt:{}:{}", projection.node_id, projection.attempt_id),
                    message: "attempt projection key does not match projection identity".to_owned(),
                });
            }
        }
        for ((run_id, cell_id), projection) in &cells {
            let (projection_node_id, projection_attempt_id) = projection.attempt_key();
            let Some(attempt) =
                attempts.get(&(projection_node_id.clone(), projection_attempt_id.clone()))
            else {
                return Err(StoreError::ProjectionConflict {
                    key: format!("cell:{run_id}:{cell_id}:terminal"),
                    message: "cell projection references missing attempt projection".to_owned(),
                });
            };
            if &attempt.run_id != run_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!("cell:{run_id}:{cell_id}:terminal"),
                    message: "cell projection key does not match attempt run id".to_owned(),
                });
            }
        }
        for (descriptor_hash, projection) in &fact_descriptors {
            if descriptor_hash != &projection.descriptor_hash {
                return Err(StoreError::ProjectionConflict {
                    key: format!("fact_descriptor:{}", projection.descriptor_hash),
                    message: "fact descriptor projection key does not match descriptor hash"
                        .to_owned(),
                });
            }
        }
        for (claim_id, projection) in &fact_query_entries {
            if claim_id != &projection.fact_claim_id {
                return Err(StoreError::ProjectionConflict {
                    key: fact_claim_projection_key("fact_query", claim_id),
                    message: "fact query projection key does not match claim id".to_owned(),
                });
            }
        }
        for ((claim_id, field_id), projection) in &fact_term_entries {
            if claim_id != &projection.fact_claim_id || field_id != &projection.field_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "{}:{}",
                        fact_claim_projection_key("fact_term", claim_id),
                        field_id
                    ),
                    message: "fact term projection key does not match projection identity"
                        .to_owned(),
                });
            }
        }
        for (ledger_ref, projection) in &side_effects {
            if ledger_ref.run_id != projection.run_id || ledger_ref.pair_id != projection.pair_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!("sidefx_pair:{}", ledger_ref.pair_id),
                    message: "side-effect projection key does not match pair reference".to_owned(),
                });
            }
            projection.ledger_state()?;
        }
        Ok(Self {
            run_states,
            run_spec_hashes,
            saga_policy_digests,
            run_completions,
            saga_engagements,
            manual_resolutions,
            attempts,
            cells,
            fact_descriptors,
            fact_query_entries,
            fact_term_entries,
            side_effects,
            resource_lanes,
            public_outputs,
            retentions,
        })
    }

    /// Validates that a loaded run stream is ordered and contiguous.
    pub fn validate_run_stream(events: &[KernelEventEnvelope]) -> Result<()> {
        validate_run_stream_order(events)?;
        validate_supported_stream_model(events)
    }

    /// Rebuilds projections from store-owned event envelopes.
    pub fn rebuild_from_run_stream(events: &[KernelEventEnvelope]) -> Result<Self> {
        Self::validate_run_stream(events)?;
        for event in events {
            match event.payload() {
                KernelEventPayload::RunAdmitted(payload)
                    if !payload.fact_descriptor_artifacts.is_empty() =>
                {
                    return Err(StoreError::ProjectionConflict {
                        key: "fact_descriptor:artifact_bytes".to_owned(),
                        message:
                            "fact descriptor projection requires retained descriptor artifact bytes"
                                .to_owned(),
                    });
                }
                KernelEventPayload::FactRecorded(_) => {
                    return Err(StoreError::ProjectionConflict {
                        key: "fact:artifact_bytes".to_owned(),
                        message: "fact projection requires retained artifact bytes".to_owned(),
                    });
                }
                _ => {}
            }
        }
        let mut snapshot = Self::default();
        let artifact_bytes = ArtifactByteAuthorityMap::new();
        for commit in committed_run_stream_commits(events) {
            let payloads = commit
                .events
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            validate_terminal_attempt_cell_pairs(&payloads)?;
            validate_fact_settlement_commit(&payloads)?;
            validate_terminal_side_effect_evidence_pairs(&payloads)?;
            validate_side_effect_attempt_failures_have_terminal_evidence(&snapshot, &payloads)?;
            validate_retention_manifest_pairs(&payloads)?;
            for event in commit.events {
                projection::apply_projection(&mut snapshot, &event, &artifact_bytes)?;
            }
        }
        Ok(snapshot)
    }

    /// Rebuilds projections from a run stream using exact retained artifact bytes.
    ///
    /// Durable stores use this for validation and physical projection rebuilds when fact descriptor
    /// and response artifacts are already loaded from authoritative storage. The supplied artifact
    /// byte authority must be keyed by exact `(artifact_id, evidence_hash)`.
    pub fn rebuild_from_run_stream_with_artifact_bytes(
        events: &[KernelEventEnvelope],
        artifact_bytes: &ArtifactByteAuthorityMap,
    ) -> Result<Self> {
        Self::validate_run_stream(events)?;
        let mut snapshot = Self::default();
        for commit in committed_run_stream_commits(events) {
            let payloads = commit
                .events
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            validate_terminal_attempt_cell_pairs(&payloads)?;
            validate_fact_settlement_commit(&payloads)?;
            validate_terminal_side_effect_evidence_pairs(&payloads)?;
            validate_side_effect_attempt_failures_have_terminal_evidence(&snapshot, &payloads)?;
            validate_retention_manifest_pairs(&payloads)?;
            for event in commit.events {
                projection::apply_projection(&mut snapshot, &event, artifact_bytes)?;
            }
        }
        Ok(snapshot)
    }

    /// Rebuilds non-fact projections plus fact record identities for stores with physical fact indexes.
    ///
    /// This helper is for stores that maintain descriptor, index, and term projections in separate
    /// validated tables. It does not rebuild queryable fact indexes from the stream, and it is not a
    /// replay validation substitute for retained descriptor and response artifact authority.
    pub fn rebuild_for_external_fact_queries(events: &[KernelEventEnvelope]) -> Result<Self> {
        Self::validate_run_stream(events)?;
        let mut snapshot = Self::default();
        for commit in committed_run_stream_commits(events) {
            let payloads = commit
                .events
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            validate_terminal_attempt_cell_pairs(&payloads)?;
            validate_fact_settlement_commit(&payloads)?;
            validate_terminal_side_effect_evidence_pairs(&payloads)?;
            validate_side_effect_attempt_failures_have_terminal_evidence(&snapshot, &payloads)?;
            validate_retention_manifest_pairs(&payloads)?;
            for event in commit.events {
                projection::apply_projection_for_external_fact_queries(&mut snapshot, &event)?;
            }
        }
        Ok(snapshot)
    }

    /// Returns the run state for a run id.
    pub fn run_state(&self, run_id: &RunId) -> RunState {
        self.run_states
            .get(run_id)
            .copied()
            .unwrap_or(RunState::Absent)
    }

    /// Returns the certified spec hash recorded at run start.
    pub fn run_spec_hash(&self, run_id: &RunId) -> Option<&SpecHash> {
        self.run_spec_hashes.get(run_id)
    }

    /// Returns the saga policy digest recorded at run start.
    pub fn saga_policy_digest(&self, run_id: &RunId) -> Option<&ContentDigest> {
        self.saga_policy_digests.get(run_id)
    }

    /// Returns the run completion projection for a run id.
    pub fn run_completion(&self, run_id: &RunId) -> Option<&RunCompletionProjection> {
        self.run_completions.get(run_id)
    }

    /// Returns the first saga engagement projection for a run id.
    pub fn saga_engagement(&self, run_id: &RunId) -> Option<&SagaEngagementProjection> {
        self.saga_engagements.get(run_id)
    }

    /// Returns the manual resolution projection for a run id.
    pub fn manual_resolution(&self, run_id: &RunId) -> Option<&ManualResolutionProjection> {
        self.manual_resolutions.get(run_id)
    }

    /// Returns a cell terminal projection.
    pub fn cell_terminal(&self, cell_id: &CellId) -> Option<&CellTerminalProjection> {
        self.cells
            .iter()
            .find_map(|((_run_id, key_cell_id), projection)| {
                (key_cell_id == cell_id).then_some(projection)
            })
    }

    /// Returns a cell terminal projection for a specific run.
    pub fn cell_terminal_for_run(
        &self,
        run_id: &RunId,
        cell_id: &CellId,
    ) -> Option<&CellTerminalProjection> {
        self.cells.get(&(run_id.clone(), cell_id.clone()))
    }

    /// Returns a descriptor catalog projection.
    pub fn fact_descriptor(
        &self,
        descriptor_hash: &ContentDigest,
    ) -> Option<&FactDescriptorProjection> {
        self.fact_descriptors.get(descriptor_hash)
    }

    /// Returns a queryable fact projection.
    pub fn fact_query_entry(
        &self,
        claim_id: &mfm_facts::FactClaimId,
    ) -> Option<&FactQueryProjection> {
        self.fact_query_entries.get(claim_id)
    }

    /// Returns an extracted fact term for a claim and field id.
    pub fn fact_term(
        &self,
        claim_id: &mfm_facts::FactClaimId,
        field_id: &mfm_facts::FactFieldId,
    ) -> Option<&FactIndexTermProjection> {
        self.fact_term_entries
            .get(&(claim_id.clone(), field_id.clone()))
    }

    /// Iterates extracted fact terms for one claim id.
    pub fn fact_terms_for_claim<'a>(
        &'a self,
        claim_id: &'a mfm_facts::FactClaimId,
    ) -> impl Iterator<Item = &'a FactIndexTermProjection> + 'a {
        self.fact_term_entries
            .iter()
            .filter(move |((term_claim_id, _field_id), _term)| term_claim_id == claim_id)
            .map(|(_key, term)| term)
    }

    /// Returns an attempt lifecycle projection.
    pub fn attempt(&self, node_id: &NodeId, attempt_id: &AttemptId) -> Option<&AttemptProjection> {
        self.attempts.get(&(node_id.clone(), attempt_id.clone()))
    }

    /// Returns the first open semantic attempt projected for a run.
    pub fn open_attempt_for_run(&self, run_id: &RunId) -> Option<&AttemptProjection> {
        self.attempts.values().find(|attempt| {
            &attempt.run_id == run_id && matches!(attempt.status, AttemptStatus::Started { .. })
        })
    }

    /// Requires that a run prefix has no open semantic attempt.
    pub fn require_no_open_semantic_attempts_for_run(&self, run_id: &RunId) -> Result<()> {
        if let Some(attempt) = self.open_attempt_for_run(run_id) {
            return Err(StoreError::ProjectionConflict {
                key: format!("run:{run_id}:attempts"),
                message: format!(
                    "manual resolution requires no open semantic attempts; attempt {}:{} is still started",
                    attempt.node_id, attempt.attempt_id
                ),
            });
        }
        Ok(())
    }

    /// Returns a side-effect projection for a run-scoped certified pair id.
    pub fn side_effect_for_pair(
        &self,
        run_id: &RunId,
        pair_id: &SideEffectPairId,
    ) -> Option<&SideEffectProjection> {
        self.side_effects.get(&SideEffectPairLedgerRef::new(
            run_id.clone(),
            pair_id.clone(),
        ))
    }

    /// Returns a validated side-effect ledger state for a run-scoped certified pair id.
    pub fn side_effect_state_for_pair(
        &self,
        run_id: &RunId,
        pair_id: &SideEffectPairId,
    ) -> Result<Option<SideEffectLedgerState<'_>>> {
        self.side_effect_for_pair(run_id, pair_id)
            .map(SideEffectProjection::ledger_state)
            .transpose()
    }

    /// Returns an active resource lane holder.
    pub fn resource_lane(&self, key: &ResourceLaneKey) -> Option<&ResourceLaneProjection> {
        self.resource_lanes.get(key)
    }

    /// Returns a public-output projection.
    pub fn public_output(
        &self,
        run_id: &RunId,
        schema_id: &SchemaId,
    ) -> Option<&PublicOutputProjection> {
        self.public_outputs
            .get(&(run_id.clone(), schema_id.clone()))
    }

    /// Returns a retention projection.
    pub fn retention(&self, run_id: &RunId) -> Option<&RetentionProjection> {
        self.retentions.get(run_id)
    }

    /// Returns whether terminal public-output authority is projected.
    pub fn has_public_output(&self, run_id: &RunId) -> bool {
        self.public_outputs
            .iter()
            .any(|((projection_run_id, _), projection)| {
                projection_run_id == run_id
                    && matches!(projection, PublicOutputProjection::Produced { .. })
            })
    }

    /// Returns whether all past-boundary forward ledgers for the current projection are quiescent.
    pub fn forward_ledgers_quiescent(
        &self,
        run_id: &RunId,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<bool> {
        forward_ledgers_quiescent(self, run_id, terminal_policies)
    }

    /// Derives saga status from certified saga policy plus the current stream projection.
    pub fn derive_saga_projection(
        &self,
        run_id: &RunId,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<SagaProjection> {
        derive_saga_projection(self, run_id, policy, terminal_policies)
    }

    /// Requires that the current prefix derives a manual-blocked saga mode.
    pub fn require_manual_resolution_admissible(
        &self,
        run_id: &RunId,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<()> {
        self.require_no_open_semantic_attempts_for_run(run_id)?;
        let saga = self.derive_saga_projection(run_id, policy, terminal_policies)?;
        if saga.run_mode == RunMode::ManualBlocked {
            Ok(())
        } else {
            Err(StoreError::ProjectionConflict {
                key: format!("run:{run_id}:manual_resolution"),
                message: format!(
                    "manual resolution requires prefix-derived manual_blocked saga mode, found {}",
                    saga.run_mode.as_str()
                ),
            })
        }
    }

    /// Returns the saga terminal outcome supported by the current prefix.
    pub fn saga_terminal_completion_outcome(
        &self,
        run_id: &RunId,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<events::RunCompletionOutcome> {
        let saga = self.derive_saga_projection(run_id, policy, terminal_policies)?;
        saga.run_mode
            .saga_terminal_outcome()
            .ok_or_else(|| StoreError::ProjectionConflict {
                key: format!("run:{run_id}:saga_terminal"),
                message: format!(
                    "saga terminal resolution requires terminal saga mode, found {}",
                    saga.run_mode.as_str()
                ),
            })
    }

    /// Iterates projected run states.
    pub fn run_states(&self) -> impl Iterator<Item = (&RunId, &RunState)> {
        self.run_states.iter()
    }

    /// Iterates run-start certified spec hashes.
    pub fn run_spec_hashes(&self) -> impl Iterator<Item = (&RunId, &SpecHash)> {
        self.run_spec_hashes.iter()
    }

    /// Iterates run-start saga policy digests.
    pub fn saga_policy_digests(&self) -> impl Iterator<Item = (&RunId, &ContentDigest)> {
        self.saga_policy_digests.iter()
    }

    /// Iterates run completion projections.
    pub fn run_completions(&self) -> impl Iterator<Item = (&RunId, &RunCompletionProjection)> {
        self.run_completions.iter()
    }

    /// Iterates saga engagement projections.
    pub fn saga_engagements(&self) -> impl Iterator<Item = (&RunId, &SagaEngagementProjection)> {
        self.saga_engagements.iter()
    }

    /// Iterates manual resolution projections.
    pub fn manual_resolutions(
        &self,
    ) -> impl Iterator<Item = (&RunId, &ManualResolutionProjection)> {
        self.manual_resolutions.iter()
    }

    /// Iterates attempt lifecycle projections.
    pub fn attempts(&self) -> impl Iterator<Item = (&(NodeId, AttemptId), &AttemptProjection)> {
        self.attempts.iter()
    }

    /// Iterates cell terminal projections.
    pub fn cells(&self) -> impl Iterator<Item = (&RunId, &CellId, &CellTerminalProjection)> {
        self.cells
            .iter()
            .map(|((run_id, cell_id), projection)| (run_id, cell_id, projection))
    }

    /// Iterates descriptor catalog projections.
    pub fn fact_descriptors(
        &self,
    ) -> impl Iterator<Item = (&ContentDigest, &FactDescriptorProjection)> {
        self.fact_descriptors.iter()
    }

    /// Iterates queryable fact projections.
    pub fn fact_query_entries(
        &self,
    ) -> impl Iterator<Item = (&mfm_facts::FactClaimId, &FactQueryProjection)> {
        self.fact_query_entries.iter()
    }

    /// Iterates extracted fact term projections.
    pub fn fact_term_entries(
        &self,
    ) -> impl Iterator<
        Item = (
            &(mfm_facts::FactClaimId, mfm_facts::FactFieldId),
            &FactIndexTermProjection,
        ),
    > {
        self.fact_term_entries.iter()
    }

    /// Iterates side-effect projections.
    pub fn side_effects(
        &self,
    ) -> impl Iterator<Item = (&SideEffectPairLedgerRef, &SideEffectProjection)> {
        self.side_effects.iter()
    }

    /// Iterates active resource lane projections.
    pub fn resource_lanes(
        &self,
    ) -> impl Iterator<Item = (&ResourceLaneKey, &ResourceLaneProjection)> {
        self.resource_lanes.iter()
    }

    /// Iterates public-output projections.
    pub fn public_outputs(
        &self,
    ) -> impl Iterator<Item = (&RunId, &SchemaId, &PublicOutputProjection)> {
        self.public_outputs
            .iter()
            .map(|((run_id, schema_id), projection)| (run_id, schema_id, projection))
    }

    /// Iterates retention projections.
    pub fn retentions(&self) -> impl Iterator<Item = (&RunId, &RetentionProjection)> {
        self.retentions.iter()
    }
}
