use super::*;

#[test]
fn persisted_certification_rejects_hash_mismatch() {
    let (certified, registry) = certifier_backed_runtime_authority();
    let persisted = certified
        .to_persisted_parts()
        .expect("persisted certification parts");
    let mut envelope: serde_json::Value =
        serde_json::from_slice(persisted.spec_bytes()).expect("persisted spec JSON");
    envelope["spec_hash"] = serde_json::Value::String(
        SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, D9).to_string(),
    );
    let serialized = serde_json::to_string(&envelope).expect("tampered persisted spec JSON");
    let tampered = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&serialized)
        .expect("canonical tampered persisted spec");

    assert!(
        mfm_certify::verify_persisted_spec_certificate(
            tampered.as_bytes(),
            persisted.certificate_bytes(),
            &registry,
        )
        .is_err(),
        "persisted hash mismatch must not mint certified authority"
    );
}

#[test]
fn certified_runtime_spec_accepts_certifier_and_verified_persisted_authority() {
    let (certified, registry) = certifier_backed_runtime_authority();
    let persisted_parts = certified
        .to_persisted_parts()
        .expect("persisted spec/certificate parts");
    let verified = mfm_certify::verify_persisted_spec_certificate(
        persisted_parts.spec_bytes(),
        persisted_parts.certificate_bytes(),
        &registry,
    )
    .expect("verified persisted spec/certificate");
    for (source, authority) in [
        ("certifier authority", certified),
        ("verified persisted parts authority", verified),
    ] {
        let runtime = CertifiedRuntimeSpec::new(authority).expect(source);
        assert!(!runtime.topological_order().is_empty(), "{source}");
    }
}

#[tokio::test]
async fn journal_load_rejects_run_completed_without_public_output_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-complete-without-public-output")
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::RunCompleted(
                events::RunCompleted {
                    run_id: fixture.run_id.clone(),
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    outcome: events::RunCompletionOutcome::Completed(Box::new(
                        events::PublicOutputCompletionEvidence {
                            public_output_schema_id: fixture
                                .runtime_spec
                                .spec()
                                .public_outputs
                                .public_schema_id
                                .clone(),
                            public_output_event_id: EventId::from_digest(
                                DigestAlgorithm::Sha256JcsV1,
                                D9,
                            ),
                        },
                    )),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged completion");
    drop(current);
    assert!(matches!(
        load_fixture_current(&scheduler, &store, &fixture).await,
        Err(RuntimeError::Store(message))
            if message.contains("RunCompleted appeared before PublicOutputProduced")
    ));
}

#[tokio::test]
async fn runner_rejects_invalid_artifact_outputs() {
    #[derive(Clone, Copy)]
    enum InvalidArtifactOutputKind {
        ForeignProducer,
        MismatchedInlineBytes,
        MissingStagedArtifact,
    }

    impl InvalidArtifactOutputKind {
        fn label(self) -> &'static str {
            match self {
                Self::ForeignProducer => "foreign producer artifact",
                Self::MismatchedInlineBytes => "mismatched inline artifact",
                Self::MissingStagedArtifact => "missing staged artifact",
            }
        }
    }

    enum InvalidArtifactOutput {
        ForeignProducer { foreign_node_id: NodeId },
        MismatchedInlineBytes,
        MissingStagedArtifact,
    }

    struct InvalidArtifactRunner {
        case: InvalidArtifactOutput,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for InvalidArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let staged_artifacts = match &self.case {
                    InvalidArtifactOutput::ForeignProducer { foreign_node_id } => {
                        let mut artifact = state_output_artifact(
                            ctx.node(),
                            ctx.descriptor(),
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        );
                        artifact.producer_node_id = Some(foreign_node_id.clone());
                        vec![staged_attempt_artifact(&ctx, artifact)?]
                    }
                    InvalidArtifactOutput::MismatchedInlineBytes => {
                        let artifact = state_output_artifact(
                            ctx.node(),
                            ctx.descriptor(),
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        );
                        vec![StagedArtifact::inline_attempt_artifact(
                            &ctx,
                            b"mismatched".to_vec(),
                            artifact,
                        )?]
                    }
                    InvalidArtifactOutput::MissingStagedArtifact => Vec::new(),
                };
                let payload_evidence = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                );
                Ok(ErasedRunnerOutput::from_parts(
                    staged_artifacts,
                    Vec::new(),
                    terminal_payloads(&ctx, &payload_evidence),
                ))
            })
        }
    }

    for kind in [
        InvalidArtifactOutputKind::ForeignProducer,
        InvalidArtifactOutputKind::MismatchedInlineBytes,
        InvalidArtifactOutputKind::MissingStagedArtifact,
    ] {
        let fixture = fixture();
        let case = match kind {
            InvalidArtifactOutputKind::ForeignProducer => InvalidArtifactOutput::ForeignProducer {
                foreign_node_id: node_by_output(&fixture, &fixture.cell_b).node_id.clone(),
            },
            InvalidArtifactOutputKind::MismatchedInlineBytes => {
                InvalidArtifactOutput::MismatchedInlineBytes
            }
            InvalidArtifactOutputKind::MissingStagedArtifact => {
                InvalidArtifactOutput::MissingStagedArtifact
            }
        };
        let registry = fixture_registry_with_first_runner(
            &fixture,
            "pure",
            InvalidArtifactRunner {
                case,
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        );
        let scheduler = fixture_scheduler(registry, &fixture);
        let mut store = TestTypedRunStore::new();
        let current = started_fixture_current(
            &scheduler,
            &mut store,
            &fixture,
            vec![fixture.seed_ref.clone()],
        )
        .await
        .expect("start invalid-artifact fixture");
        let result = drive_current_once_with_claim(&scheduler, &store, current)
            .await
            .unwrap_or_else(|_| panic!("{}", kind.label()));
        assert_eq!(
            result.status(),
            SchedulerStatus::Advanced,
            "{}",
            kind.label()
        );
        let current = result.into_current_run();
        let node = node_by_output(&fixture, &fixture.cell_a);
        assert_node_failed_with_code(&current, &node.node_id, "runner_output_invalid");
    }
}

#[tokio::test]
async fn runner_can_commit_inline_state_output_artifact() {
    struct InlineArtifactRunner {
        output_bytes: Vec<u8>,
    }

    impl ErasedNodeRunner for InlineArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    Vec::new(),
                    terminal_payloads(&ctx, &artifact),
                ))
            })
        }
    }

    let fixture = fixture();
    let output_bytes = br#"{"inline":true}"#.to_vec();
    let output_digest = digest_for_bytes(&output_bytes);
    let output_artifact =
        ArtifactId::from_digest(output_digest.algorithm(), *output_digest.digest());
    let registry =
        fixture_registry_with_first_runner(&fixture, "pure", InlineArtifactRunner { output_bytes });
    let scheduler = fixture_scheduler(registry, &fixture);
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start inline-artifact fixture");
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("drive inline output");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    let current = result.into_current_run();
    let produced = current
        .lifecycle()
        .cell(&fixture.cell_a)
        .expect("inline output cell")
        .produced()
        .expect("produced inline output");
    assert_eq!(produced.artifact_id(), &output_artifact);
    assert_eq!(produced.content_digest(), &output_digest);
}

#[tokio::test]
async fn runner_cannot_stage_reserved_retention_reasons() {
    struct ReservedRetentionReasonRunner {
        output_bytes: Vec<u8>,
        reason: events::RetentionReason,
    }

    impl ErasedNodeRunner for ReservedRetentionReasonRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    vec![StagedRetentionRefs {
                        refs: vec![artifact.retention_ref()?],
                        reason: self.reason,
                        authority:
                            crate::artifacts::StagedRetentionRefAuthority::CurrentCommitArtifacts,
                    }],
                    terminal_payloads(&ctx, &artifact),
                ))
            })
        }
    }

    for reason in [
        events::RetentionReason::RunAdmitted,
        events::RetentionReason::ManifestProjection,
        events::RetentionReason::PublicOutput,
    ] {
        let fixture = fixture();
        let node = node_by_output(&fixture, &fixture.cell_a).clone();
        let registry = fixture_registry_with_first_runner(
            &fixture,
            "pure",
            ReservedRetentionReasonRunner {
                output_bytes: br#"{"reserved":true}"#.to_vec(),
                reason,
            },
        );
        let scheduler = fixture_scheduler(registry, &fixture);
        let mut store = TestTypedRunStore::new();
        let current = started_fixture_current(
            &scheduler,
            &mut store,
            &fixture,
            vec![fixture.seed_ref.clone()],
        )
        .await
        .expect("start reserved-retention fixture");
        let mut records_before = 0;
        let _ = current.lifecycle().visit_records(|_| {
            records_before += 1;
            std::ops::ControlFlow::<()>::Continue(())
        });
        let result = drive_current_once_with_claim(&scheduler, &store, current)
            .await
            .expect("terminalize reserved retention reason");
        assert_eq!(result.status(), SchedulerStatus::Advanced);
        let current = result.into_current_run();
        let mut records_after = 0;
        let _ = current.lifecycle().visit_records(|_| {
            records_after += 1;
            std::ops::ControlFlow::<()>::Continue(())
        });
        assert_eq!(records_after, records_before + 4);
        assert_node_failed_with_code(&current, &node.node_id, "runner_output_invalid");
        assert!(current.lifecycle().cell(&fixture.cell_a).is_none());
    }
}

#[tokio::test]
async fn rejected_staged_payload_mismatch_does_not_admit_artifact_evidence() {
    struct MismatchedStagedArtifactRunner {
        staged_artifact: ArtifactId,
        staged_digest: ContentDigest,
        payload_artifact: ArtifactId,
        payload_digest: ContentDigest,
    }

    impl ErasedNodeRunner for MismatchedStagedArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.staged_artifact.clone(),
                    self.staged_digest.clone(),
                );
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                let payload_evidence = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.payload_artifact.clone(),
                    self.payload_digest.clone(),
                );
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    Vec::new(),
                    terminal_payloads(&ctx, &payload_evidence),
                ))
            })
        }
    }

    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    let staged_artifact = artifact(0xa1);
    let staged_digest = content(0xa2);
    let payload_artifact = artifact(0xa3);
    let payload_digest = content(0xa4);
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        MismatchedStagedArtifactRunner {
            staged_artifact: staged_artifact.clone(),
            staged_digest: staged_digest.clone(),
            payload_artifact,
            payload_digest,
        },
    );
    let scheduler = fixture_scheduler(registry, &fixture);
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start staged-mismatch fixture");
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");

    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("terminalize staged payload mismatch");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    let current = result.into_current_run();
    assert_node_failed_with_code(&current, &node.node_id, "runner_output_invalid");

    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(&node)
        .expect("descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let attempt_logical_key =
        store::LogicalEventKey::new(format!("attempt:{}:{}", node.node_id, attempt_id))
            .expect("attempt key");
    let leaked_artifact_request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new("missing-leaked-staged-artifact").expect("commit key"),
        payloads: vec![
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                cell_id: node.output_cell.clone(),
                scope_id: node.scope_id.clone(),
                attempt_id: attempt_id.clone(),
                semantic_type_id: descriptor.output_semantic_type_id.clone(),
                schema_id: descriptor.output_schema_id.clone(),
                value_lineage: output_cell.value_lineage.clone(),
                context: output_cell.context.clone(),
                artifact_id: staged_artifact.clone(),
                content_digest: staged_digest.clone(),
                evidence_hash: staged_digest,
                producer_state_kind: Some(node.state_kind.clone()),
                producer_state_version: Some(node.state_version.clone()),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                output_cell_id: node.output_cell.clone(),
            }),
        ],
        required_artifacts: Vec::new(),
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_present_logical_keys: vec![attempt_logical_key],
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    };
    let artifacts = store::CommitArtifactEvidenceSet::new(
        leaked_artifact_request.required_artifacts().to_vec(),
        Vec::new(),
    )
    .expect("leak probe artifact evidence set");
    let error =
        store::PreparedCommit::<store::AttemptTerminal>::new(leaked_artifact_request, artifacts)
            .expect_err("missing leak evidence rejects before append");
    assert!(matches!(
        error,
        store::StoreError::InvalidPreparedCommitPurpose { message, .. }
            if message.contains("missing required artifact evidence")
                && message.contains(staged_artifact.as_str())
    ));
}
