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

#[test]
fn materialization_rejects_seed_digest_not_certified() {
    let fixture = fixture();
    let mut seed = fixture.seed_ref.clone();
    seed.digest = content(0xee);
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    assert!(matches!(
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![seed],),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[test]
fn run_start_rejects_missing_config_artifact_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence.config_artifacts.clear();
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[test]
fn run_start_rejects_missing_fact_descriptor_artifact_evidence() {
    let (fixture, _descriptor, descriptor_ref) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);

    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains("missing fact descriptor artifact")
                && message.contains(descriptor_ref.descriptor_hash.as_str())
    ));
}

#[tokio::test]
async fn run_start_admits_certified_fact_descriptor_artifacts() {
    let (fixture, descriptor, descriptor_ref) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));

    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");

    let run_admitted = store.run_admitted(&fixture.run_id);
    assert_eq!(run_admitted.fact_descriptor_artifacts.len(), 1);
    let admitted_descriptor = &run_admitted.fact_descriptor_artifacts[0];
    assert_eq!(
        admitted_descriptor.role,
        events::ArtifactRole::FactDescriptor
    );
    assert_eq!(
        admitted_descriptor.content_digest,
        descriptor_ref.descriptor_hash
    );
    let committed = block_on_ready(store.load_committed_run_stream(&fixture.run_id))
        .expect("descriptor-backed committed stream");
    RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("descriptor-backed run stream validates");
}

#[tokio::test]
async fn raw_runtime_view_rejects_fact_descriptor_stream_without_artifact_authority() {
    let (fixture, descriptor, _) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));

    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");

    let stream = store.load_run_stream(&fixture.run_id);
    let error = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect_err("raw descriptor-bearing stream must fail closed");
    assert!(matches!(
        error,
        RuntimeError::InvalidRunStream(message)
            if message.contains("committed stream artifact authority")
    ));
}

#[tokio::test]
async fn run_start_admitted_uses_committed_fact_descriptor_artifacts() {
    let (fixture, descriptor, _) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");

    let authority =
        scheduler_start_run_admitted(&scheduler, &mut store, &fixture.runtime_spec, launch)
            .await
            .expect("descriptor-backed admission uses committed stream authority");

    assert_eq!(authority.run_id(), &fixture.run_id);
    assert_eq!(
        authority.head_seq(),
        store.expected_next_seq(&fixture.run_id)
    );
}

#[tokio::test]
async fn fact_bearing_runtime_prefix_rebuild_uses_retained_artifact_bytes() {
    let (fixture, descriptor, _) = fixture_with_read_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");
    let node_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let node_a_attempt = append_attempt_start(&mut store, &fixture, &node_a, 1);
    append_terminal(
        &mut store,
        &fixture,
        &node_a,
        &node_a_attempt,
        artifact(0x70),
        content(0x71),
    );
    let node = node_by_output(&fixture, &fixture.cell_b).clone();
    let attempt_id = append_attempt_start(&mut store, &fixture, &node, 1);
    append_fact(&mut store, &fixture, &node, &attempt_id, 17, 23);

    let stream = store.load_run_stream(&fixture.run_id);
    let raw_error = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect_err("raw fact-bearing stream must fail closed");
    assert!(matches!(
        raw_error,
        RuntimeError::InvalidRunStream(message)
            if message.contains("committed stream artifact authority")
    ));
    let committed = block_on_ready(store.load_committed_run_stream(&fixture.run_id))
        .expect("fact-bearing committed stream");
    RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("fact-bearing runtime view validates with retained bytes");
    build_retention_manifest_artifact(
        &fixture.runtime_spec,
        &fixture.run_id,
        &stream,
        committed.artifact_byte_authority(),
    )
    .expect("fact-bearing prefix rebuilds with retained bytes");

    let missing = store::ArtifactByteAuthorityMap::new();
    build_retention_manifest_artifact(&fixture.runtime_spec, &fixture.run_id, &stream, &missing)
        .expect_err("fact-bearing prefix without retained bytes must fail");

    let (response_evidence, _) = test_fact_response_artifact(&node, 23);
    let response_key = (
        response_evidence.artifact_id.clone(),
        response_evidence
            .evidence_hash()
            .expect("response evidence hash"),
    );
    let mut mismatched = committed.artifact_byte_authority().clone();
    mismatched
        .get_mut(&response_key)
        .expect("response bytes retained")
        .0 = b"{\"amount\":999}".to_vec();
    build_retention_manifest_artifact(&fixture.runtime_spec, &fixture.run_id, &stream, &mismatched)
        .expect_err("fact-bearing prefix with mismatched response bytes must fail");
}

#[test]
fn run_start_rejects_mismatched_staged_launch_bytes() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let base = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);

    let mut bad_spec = base.clone();
    bad_spec.spec_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_spec,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_certificate = base.clone();
    bad_certificate.certificate_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_certificate,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_config = base.clone();
    bad_config
        .config_artifacts
        .first_mut()
        .expect("config artifact")
        .bytes
        .push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_config,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_seed = base;
    bad_seed
        .seed_cells
        .first_mut()
        .expect("seed cell")
        .bytes
        .push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_seed,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));
    assert!(store.load_run_stream(&fixture.run_id).is_empty());
}

#[tokio::test]
async fn run_admission_returns_bound_context_with_capability_and_framework_authority() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let launch =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .expect("prepare launch");

    let authority =
        scheduler_start_run_admitted(&scheduler, &mut store, &fixture.runtime_spec, launch)
            .await
            .expect("admitted run authority");

    assert_eq!(authority.run_id(), &fixture.run_id);
    assert_eq!(authority.spec_hash(), fixture.runtime_spec.spec_hash());
    assert_eq!(
        authority.head_seq(),
        store.expected_next_seq(&fixture.run_id)
    );

    let node = node_by_output(&fixture, &fixture.cell_b);
    let capability = authority
        .bound_context()
        .capability_authority_for(&node.node_id)
        .expect("capability authority");
    assert_eq!(capability.capabilities(), &node.capability_bindings);
    assert_eq!(capability.implementations().len(), 1);
    assert_eq!(
        capability.implementations()[0].descriptor(),
        &node.capability_bindings.capabilities[0]
    );
    assert_eq!(
        capability.implementations()[0].implementation_id().as_str(),
        "mfm.test.capability"
    );

    let render_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        })
        .expect("public-output render node");
    let framework = authority
        .bound_context()
        .framework_handler_for(&render_node.node_id)
        .expect("framework handler authority");
    assert_eq!(
        framework.kind(),
        BoundFrameworkHandlerKind::PublicOutputRender
    );

    let run_admitted = store.run_admitted(&fixture.run_id);
    assert_eq!(
        run_admitted.runner_executables,
        authority.bound_context().runner_executables()
    );
    assert!(
        !authority.bound_context().adapter_executables().is_empty(),
        "fixture must exercise adapter executable binding evidence"
    );
    assert_eq!(
        run_admitted.adapter_executables,
        authority.bound_context().adapter_executables()
    );
    scheduler
        .validate_admitted_run_binding(&fixture.runtime_spec, &run_admitted)
        .expect("binding validation");
}

#[test]
fn run_start_rejects_invalid_capability_implementation_bindings() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        MissingImplementation,
        DescriptorMismatch,
    }

    for (case, expected_message) in [
        (
            Case::MissingImplementation,
            "missing capability implementation",
        ),
        (
            Case::DescriptorMismatch,
            "differs from certified descriptor",
        ),
    ] {
        let fixture = fixture();
        let registry = match case {
            Case::MissingImplementation => fixture_registry_with_first_runner(
                &fixture,
                "pure",
                RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ),
            Case::DescriptorMismatch => {
                let mut registry = ErasedRunnerRegistry::new();
                let implementation_id = CapabilityImplementationId::new("mfm.test.capability")
                    .expect("capability implementation");
                registry
                    .register_capability(CapabilityImplementationBinding::new(
                        CapabilityDescriptor::new(
                            fixture.cap_kind.clone(),
                            fixture.cap_version.clone(),
                            CapabilityRole::ReadExternal,
                            "wrong-read-db",
                        )
                        .expect("wrong capability descriptor"),
                        implementation_id,
                    ))
                    .expect("capability implementation");
                register_default_fixture_pure_runner(&mut registry, &fixture);
                register_default_fixture_read_runner(&mut registry, &fixture);
                registry
            }
        };
        let scheduler = test_scheduler(registry);
        let store = TestTypedRunStore::new();
        let error =
            prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
                .err()
                .expect("invalid capability implementation binding must reject launch");
        assert!(
            matches!(&error, RuntimeError::RunnerBinding(message) if message.contains(expected_message)),
            "{case:?} returned unexpected error: {error:?}"
        );
    }
}

#[tokio::test]
async fn resume_rejects_binding_changes_before_attempt_start() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        MissingDownstreamBinding,
        RunnerExecutableMismatch,
        AdapterExecutableMismatch,
    }

    for (case, expected_message, direct_validation_rejects) in [
        (
            Case::MissingDownstreamBinding,
            "missing runner binding",
            false,
        ),
        (
            Case::RunnerExecutableMismatch,
            "runner executable identities",
            true,
        ),
        (
            Case::AdapterExecutableMismatch,
            "adapter executable identities",
            true,
        ),
    ] {
        let fixture = fixture();
        let (_, mut store) = started_fixture_run(&fixture).await;
        let stream_len_before = store.load_run_stream(&fixture.run_id).len();

        let resume_scheduler = match case {
            Case::MissingDownstreamBinding => {
                let mut partial_registry = ErasedRunnerRegistry::new();
                register_default_fixture_pure_runner(&mut partial_registry, &fixture);
                test_scheduler(partial_registry)
            }
            Case::RunnerExecutableMismatch => {
                let mut changed_registry = ErasedRunnerRegistry::new();
                let mut changed_a = binding(
                    fixture.descriptor_a.clone(),
                    "pure",
                    RecordingRunner {
                        expected_caps: Vec::new(),
                        output_artifact: artifact(0xa1),
                        output_digest: content(0xa2),
                    },
                );
                changed_a.executable.binary_digest = content(0xee);
                changed_registry.register(changed_a).expect("binding a");
                register_default_fixture_read_runner(&mut changed_registry, &fixture);
                fixture_scheduler(changed_registry, &fixture)
            }
            Case::AdapterExecutableMismatch => {
                let mut changed_adapter = test_adapter_executable_identity();
                changed_adapter.binary_digest = content(0xef);
                test_scheduler(registered_fixture_runners_with_adapter_executable(
                    &fixture,
                    changed_adapter,
                ))
            }
        };

        if direct_validation_rejects {
            let run_admitted = store.run_admitted(&fixture.run_id);
            resume_scheduler
                .validate_admitted_run_binding(&fixture.runtime_spec, &run_admitted)
                .expect_err("binding validation should reject");
        }

        let error = drive_fixture_once(&resume_scheduler, &mut store, &fixture)
            .await
            .expect_err("changed binding should reject bound context");
        assert!(
            matches!(&error, RuntimeError::RunnerBinding(message) if message.contains(expected_message)),
            "{case:?} returned unexpected error: {error:?}"
        );
        store.assert_run_stream_len(&fixture.run_id, stream_len_before);
    }
}

#[tokio::test]
async fn runner_invocation_uses_run_admitted_config_evidence_without_reference_event() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let valid_stream = store.load_run_stream(&fixture.run_id);
    assert!(valid_stream.iter().all(|event| {
        !matches!(
            event.payload(),
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::TypedConfig
        )
    }));
    drive_ok!(
        scheduler,
        store,
        fixture,
        "drive with RunAdmitted config evidence"
    );
}

#[tokio::test]
async fn runner_invocation_requires_committed_produced_input_artifact_reference() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                    && payload.node_id.as_ref() == Some(&producer_node_id)
        )
    });
    {
        let corrupt_store = StaleStreamStore::new(&mut store, corrupt_stream);
        assert!(matches!(
            drive_once_with_claim(&scheduler, &corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InputMaterialization(message))
                if message.contains("is not committed in the run stream")
        ));
    }
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &consumer_node_id),
        1
    );
}

#[tokio::test]
async fn post_start_materialization_failure_terminalizes_attempt() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node = node_by_output(&fixture, &fixture.cell_b).clone();
    {
        let corrupt_store = MissingInputArtifactRefStore::new(&mut store, producer_node_id);
        assert_eq!(
            drive_once_with_claim(
                &scheduler,
                &corrupt_store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("terminalize materialization failure"),
            SchedulerStatus::Advanced
        );
    }

    assert_node_failed_with_code_and_retryable(
        &store,
        &consumer_node.node_id,
        "input_materialization_failed",
        true,
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&consumer_node.output_cell)
        .is_none());
}

#[tokio::test]
async fn post_start_runner_errors_follow_terminal_policy_by_error_class() {
    #[derive(Clone, Copy)]
    enum Case {
        RuntimeValidation,
        InvalidRunStream,
    }

    for case in [Case::RuntimeValidation, Case::InvalidRunStream] {
        let fixture = fixture();
        let error = match case {
            Case::RuntimeValidation => RuntimeError::RuntimeValidation(
                "synthetic post-start validation failure".to_owned(),
            ),
            Case::InvalidRunStream => {
                RuntimeError::InvalidRunStream("synthetic corrupt stream authority".to_owned())
            }
        };
        let registry = fixture_registry_with_first_runner(&fixture, "pure", ErrorRunner { error });
        let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

        match case {
            Case::RuntimeValidation => {
                assert_drive!(
                    scheduler,
                    store,
                    fixture,
                    Advanced,
                    "terminalize runtime validation failure"
                );

                let node = node_by_output(&fixture, &fixture.cell_a);
                assert_node_failed_with_code(&store, &node.node_id, "runtime_validation_failed");
                assert_eq!(
                    runtime_lifecycle_summary(&store, &fixture.run_id),
                    "run=Started attempts[started=0 completed=0 failed=1 interrupted=0 total=1] cells=0 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
                );
            }
            Case::InvalidRunStream => {
                assert!(matches!(
                    drive_fixture_once(&scheduler, &mut store, &fixture)
                        .await,
                    Err(RuntimeError::InvalidRunStream(message))
                        if message.contains("synthetic corrupt stream authority")
                ));

                let node = node_by_output(&fixture, &fixture.cell_a);
                let projection_snapshot = store.projection_snapshot();
                let attempts = projection_snapshot
                    .attempts()
                    .filter(|((node_id, _), _)| node_id == &node.node_id)
                    .map(|(_, attempt)| attempt)
                    .collect::<Vec<_>>();
                assert_eq!(attempts.len(), 1);
                assert!(matches!(
                    attempts[0].status,
                    store::AttemptStatus::Started { .. }
                ));
                assert_failure_code_count(&store, "runtime_validation_failed", 0);
                assert_failure_code_count(&store, "runner_output_invalid", 0);
                assert_eq!(
                    runtime_lifecycle_summary(&store, &fixture.run_id),
                    "run=Started attempts[started=1 completed=0 failed=0 interrupted=0 total=1] cells=0 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
                );
            }
        }
    }
}

#[tokio::test]
async fn replay_rejects_terminal_cell_producer_outside_certified_spec() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let forged_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell != fixture.cell_a)
        .expect("second node")
        .clone();
    let certified_cell = fixture
        .runtime_spec
        .cell(&fixture.cell_a)
        .expect("cell a")
        .clone();
    let forged_attempt = AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xfa; 32]),
    );
    let artifact_id = artifact(0xfa);
    let artifact_digest = content(0xfb);
    let forged_artifact = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: artifact_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(certified_cell.schema_id.clone()),
        semantic_type_id: Some(certified_cell.semantic_type_id.clone()),
        producer_node_id: Some(forged_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-attempt-start").expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    attempt_no: 1,
                    state_kind: forged_node.state_kind.clone(),
                    state_version: forged_node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged attempt start");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-terminal").expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    cell_id: fixture.cell_a.clone(),
                    scope_id: certified_cell.scope_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    semantic_type_id: certified_cell.semantic_type_id.clone(),
                    schema_id: certified_cell.schema_id.clone(),
                    value_lineage: certified_cell.value_lineage.clone(),
                    context: certified_cell.context.clone(),
                    artifact_id,
                    content_digest: artifact_digest,
                    evidence_hash: forged_artifact
                        .evidence_hash()
                        .expect("forged terminal evidence hash"),
                    producer_state_kind: Some(forged_node.state_kind.clone()),
                    producer_state_version: Some(forged_node.state_version.clone()),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    attempt_id: forged_attempt,
                    output_cell_id: fixture.cell_a.clone(),
                }),
            ],
            required_artifacts: vec![forged_artifact],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged terminal");

    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture).await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}
