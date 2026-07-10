use super::*;

impl PostgresRunStore {
    /// Atomically admits artifact evidence and appends one typed run commit.
    pub async fn append_prepared_commit_bundle(
        &self,
        bundle: PreparedCommitBundle,
    ) -> Result<CommitOutcome> {
        let plan = bundle.plan();
        let request = plan.request();
        let fingerprint = prepared_commit_plan_fingerprint(plan)?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| database_error("failed to start transaction", error))?;

        if let Some(stored) =
            read_commit_by_key(&mut tx, request.run_id(), request.commit_key().as_str()).await?
        {
            if stored.prepared_commit_plan_fingerprint == fingerprint {
                let batch = load_committed_batch_tx(&mut tx, request.run_id(), stored.seq).await?;
                tx.commit()
                    .await
                    .map_err(|_| PostgresStoreError::Database("failed to commit transaction"))?;
                return Ok(CommitOutcome::Idempotent(batch));
            }
            return Err(StoreError::CommitConflict {
                commit_key: request.commit_key().clone(),
            }
            .into());
        }

        lock_run_tx(&mut tx, request.run_id()).await?;
        let head = read_head_tx(&mut tx, request.run_id()).await?;

        // A same-run transaction may have inserted the key while this transaction waited for the
        // run lock. Re-check before sequence validation while preserving the required initial
        // commit-key lookup order.
        if let Some(stored) =
            read_commit_by_key(&mut tx, request.run_id(), request.commit_key().as_str()).await?
        {
            if stored.prepared_commit_plan_fingerprint == fingerprint {
                let batch = load_committed_batch_tx(&mut tx, request.run_id(), stored.seq).await?;
                tx.commit()
                    .await
                    .map_err(|_| PostgresStoreError::Database("failed to commit transaction"))?;
                return Ok(CommitOutcome::Idempotent(batch));
            }
            return Err(StoreError::CommitConflict {
                commit_key: request.commit_key().clone(),
            }
            .into());
        }

        verify_prepared_artifact_bundle_tx(&mut tx, &bundle).await?;
        let mut artifacts = load_artifacts(&mut tx, request.run_id()).await?;
        mfm_store::v1::backend::admit_artifact_evidence(
            &mut artifacts,
            bundle.admitted_artifacts(),
        )?;
        let (run_projection, stream_head) =
            rebuild_projection_snapshot_with_head(&mut tx, request.run_id()).await?;
        if stream_head != head {
            return Err(PostgresStoreError::Corruption(
                "run commit head does not match persisted event stream".to_owned(),
            ));
        }
        let locked_resource_admission_lanes =
            lock_resource_lanes_for_request_tx(&mut tx, request, &run_projection).await?;
        for lane in locked_resource_admission_lanes.values() {
            expire_wait_fifo_waiters_tx(&mut tx, lane).await?;
        }
        let resource_lane_state = load_resource_lane_state_tx(&mut tx).await?;
        let projections =
            projection_snapshot_with_resource_lanes(&run_projection, resource_lane_state.active)?;
        let mut artifact_bytes = mfm_store::v1::backend::artifact_byte_authority_for_bundle(
            &mfm_store::v1::ArtifactByteAuthorityMap::new(),
            &bundle,
        )?;
        load_fact_descriptor_artifact_bytes_tx(
            &mut tx,
            request.run_id(),
            &projections,
            &mut artifact_bytes,
        )
        .await?;
        let claim_admission = single_lane_claim_admission(request)?;
        let mut base = CommitBase {
            artifacts,
            artifact_bytes,
            logical_keys: load_logical_keys(&mut tx, request.run_id()).await?,
            unique_logical_payloads: load_unique_logical_payloads(&mut tx, request.run_id())
                .await?,
            projections: projections.clone(),
            resource_lane_authority: resource_lane_state.authority,
            actual_next_seq: next_seq_from_head(head)?,
            store_commit_order: StoreCommitOrder::EMPTY,
        };
        // Validate and assemble the run-local mutation before contending on the global ordering
        // row. The order is only needed while materializing the final envelopes and projections.
        base.store_commit_order = next_store_commit_order_tx(&mut tx).await?;
        let staged = match stage_prepared_commit_plan(&base, plan)? {
            StagedCommitOutcome::Staged(staged) => *staged,
            StagedCommitOutcome::AdmissionBlocked(block) => {
                if let Some(admission) = &claim_admission {
                    let mut block = *block;
                    block.waiter = Some(
                        enqueue_or_refresh_wait_fifo_waiter_tx(
                            &mut tx,
                            &admission.lane,
                            &admission.admission_token,
                        )
                        .await?,
                    );
                    tx.commit().await.map_err(|_| {
                        PostgresStoreError::Database("failed to commit transaction")
                    })?;
                    return Ok(CommitOutcome::AdmissionBlocked(Box::new(block)));
                }
                return Ok(CommitOutcome::AdmissionBlocked(block));
            }
        };
        if let Some(claim) = bundle.execution_claim() {
            if let Some(busy) = execution_claim_admission_pre_gate_tx(&mut tx, claim).await? {
                return Ok(CommitOutcome::ExecutionClaimBusy(Box::new(busy)));
            }
        }
        if let Some(admission) = &claim_admission {
            if let Some(block) = resource_lane_fifo_pre_gate_tx(&mut tx, admission).await? {
                tx.commit()
                    .await
                    .map_err(|_| PostgresStoreError::Database("failed to commit transaction"))?;
                return Ok(CommitOutcome::AdmissionBlocked(Box::new(block)));
            }
        }
        let batch = staged.batch().clone();
        advance_store_commit_order_tx(&mut tx, batch.store_commit_order()).await?;
        let commit_id = mfm_store::v1::backend::derive_commit_id(
            request.run_id(),
            batch.seq(),
            request.commit_key(),
            plan.purpose_name(),
            &fingerprint,
        )?;
        let final_authority = final_commit_authority(plan, &bundle, &batch, &commit_id)?;

        let commit_seq = u64_to_i64(batch.seq().as_u64(), "commits.seq")?;
        let event_count = i32::try_from(batch.events().len())
            .map_err(|_| PostgresStoreError::Corruption("event count overflow".into()))?;
        sqlx::query(
            "INSERT INTO commits \
            (commit_id, run_id, seq, commit_key, commit_purpose, prepared_commit_plan_fingerprint, \
              commit_batch_hash, store_commit_order, event_count) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(&commit_id)
        .bind(request.run_id().as_str())
        .bind(commit_seq)
        .bind(request.commit_key().as_str())
        .bind(plan.purpose_name())
        .bind(fingerprint.as_digest().as_str())
        .bind(&final_authority.commit_batch_hash)
        .bind(u64_to_i64(
            batch.store_commit_order().as_u64(),
            "commits.store_commit_order",
        )?)
        .bind(event_count)
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to insert commit authority", error))?;

        admit_artifact_bundle_tx(
            &mut tx,
            request.run_id(),
            request.commit_key(),
            batch.seq(),
            &commit_id,
            &bundle,
        )
        .await?;

        for event in batch.events() {
            let payload_canonical_json = payload_canonical_bytes(event.payload())?;
            let seq = u64_to_i64(event.seq().as_u64(), "run_events.seq")?;
            let ordinal = i32::try_from(event.ordinal().as_u32()).map_err(|_| {
                PostgresStoreError::Corruption("run_events.ordinal overflow".into())
            })?;
            sqlx::query(
                "INSERT INTO run_events \
                 (run_id, seq, ordinal, commit_id, event_id, event_schema_id, spec_hash, commit_key, \
                  logical_key, payload_hash, payload_canonical_json) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            )
            .bind(event.run_id().as_str())
            .bind(seq)
            .bind(ordinal)
            .bind(&commit_id)
            .bind(event.event_id().as_str())
            .bind(event.event_schema_id().as_str())
            .bind(event.spec_hash().as_str())
            .bind(event.commit_key().as_str())
            .bind(event.logical_key().as_str())
            .bind(event.payload_hash().as_str())
            .bind(payload_canonical_json.as_bytes())
            .execute(&mut *tx)
            .await
            .map_err(|error| database_error("failed to insert run event", error))?;
        }
        insert_fact_projection_rows_tx(
            &mut tx,
            &base.projections,
            staged.projections(),
            batch.events(),
            &commit_id,
        )
        .await?;
        if let Some(admission) = &claim_admission {
            mark_wait_fifo_waiter_admitted_tx(&mut tx, &admission.lane, &admission.admission_token)
                .await?;
        }
        notify_observation_change_tx(&mut tx).await?;

        tx.commit()
            .await
            .map_err(|error| database_error("failed to commit transaction", error))?;
        Ok(CommitOutcome::Appended(batch))
    }
}
