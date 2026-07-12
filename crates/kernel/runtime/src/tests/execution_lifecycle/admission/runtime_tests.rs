use super::*;

#[test]
fn certified_runtime_spec_rejects_hash_mismatch() {
    let fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    envelope.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
    assert!(matches!(
        CertifiedRuntimeSpec::from_verified_envelope(envelope),
        Err(RuntimeError::SpecHash(_))
    ));
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
async fn replay_rejects_run_completed_without_public_output_evidence() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
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

    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("RunCompleted appeared before PublicOutputProduced")
    ));
}

#[tokio::test]
async fn scheduler_rejects_uncertified_capability_use() {
    struct BadFactRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
    }

    impl ErasedNodeRunner for BadFactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let (response_evidence, _response_bytes) =
                    test_fact_response_artifact(ctx.node(), 194);
                Ok(ErasedRunnerOutput::new(vec![
                    RunnerEventPayload::FactRecorded(RunnerFactRecorded::new(
                        events::FactRecorded {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            claim: test_fact_claim(
                                196,
                                ctx.node().config_ref.schema_id.clone(),
                                content(0xc1),
                                &response_evidence,
                                self.cap_kind.clone(),
                                self.cap_version.clone(),
                                AdapterKind::new(
                                    "mfm.test",
                                    "adapter",
                                    DigestAlgorithm::Sha256JcsV1,
                                    D1,
                                )
                                .expect("adapter"),
                                AdapterVersion::new("mfm.adapter.v1").expect("adapter version"),
                            ),
                        },
                    )),
                ]))
            })
        }
    }

    let fixture = fixture();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        BadFactRunner {
            cap_kind: fixture.cap_kind.clone(),
            cap_version: fixture.cap_version.clone(),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        fixture,
        "terminalize uncertified capability use"
    );
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
        let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

        assert_eq!(
            drive_fixture_once(&scheduler, &mut store, &fixture)
                .await
                .unwrap_or_else(|_| panic!("{}", kind.label())),
            SchedulerStatus::Advanced,
            "{}",
            kind.label()
        );
        let node = node_by_output(&fixture, &fixture.cell_a);
        assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
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
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_drive!(scheduler, store, fixture, Advanced, "drive inline output");
    assert!(matches!(
        store.projection_snapshot().cell_terminal(&fixture.cell_a),
        Some(store::CellTerminalProjection::Produced {
            artifact_id,
            content_digest,
            ..
        }) if artifact_id == &output_artifact && content_digest == &output_digest
    ));
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
        let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
        let stream_before = store.load_run_stream(&fixture.run_id);

        assert_drive!(
            scheduler,
            store,
            fixture,
            Advanced,
            "terminalize reserved retention reason"
        );
        store.assert_run_stream_len(&fixture.run_id, stream_before.len() + 4);
        assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none());
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
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "terminalize staged payload mismatch"
    );
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");

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
