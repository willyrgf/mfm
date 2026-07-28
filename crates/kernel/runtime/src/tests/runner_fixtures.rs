use super::*;

pub(super) struct RecordingRunner {
    pub(super) expected_caps: Vec<(CapabilityKind, CapabilityVersion)>,
    pub(super) output_digest: ContentDigest,
}

impl ErasedNodeRunner for RecordingRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            for (kind, version) in &self.expected_caps {
                assert!(ctx.caps().contains(kind, version));
            }
            let output_cell = ctx.node().output_cell.clone();
            let cell = match ctx.inputs().root.clone() {
                MaterializedInputNode::Cell(cell) => cell,
                MaterializedInputNode::Unit
                | MaterializedInputNode::Tuple(_)
                | MaterializedInputNode::Struct(_)
                | MaterializedInputNode::Vec(_)
                | MaterializedInputNode::NonEmptyVec(_) => {
                    panic!("expected cell input")
                }
            };
            assert!(matches!(
                cell.terminal,
                MaterializedCellTerminal::Seed { .. } | MaterializedCellTerminal::Produced { .. }
            ));
            let certified_cell = ctx.lifecycle().cell(&output_cell).is_none();
            assert!(certified_cell);
            let output_bytes = synthetic_artifact_bytes_for_digest(&self.output_digest);
            let output_artifact = ArtifactId::from_digest(
                self.output_digest.algorithm(),
                *self.output_digest.digest(),
            );
            let artifact = store::ArtifactEvidenceRef {
                artifact_id: output_artifact.clone(),
                digest: self.output_digest.clone(),
                byte_len: output_bytes.len() as u64,
                media_type: spec::MediaType::new("application/json").expect("media"),
                schema_id: Some(ctx.descriptor().output_schema_id.clone()),
                semantic_type_id: Some(ctx.descriptor().output_semantic_type_id.clone()),
                producer_node_id: Some(ctx.node().node_id.clone()),
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::StateOutput,
            };
            let evidence_hash = artifact
                .evidence_hash()
                .expect("recording runner state output evidence hash");
            let staged_artifact =
                StagedArtifact::inline_attempt_artifact(&ctx, output_bytes, artifact)?;
            Ok(ErasedRunnerOutput::from_parts(
                vec![staged_artifact],
                Vec::new(),
                vec![RunnerEventPayload::CellProduced(events::CellProduced {
                    spec_hash: ctx.spec_hash().clone(),
                    node_id: ctx.node().node_id.clone(),
                    cell_id: ctx.node().output_cell.clone(),
                    scope_id: ctx.node().scope_id.clone(),
                    attempt_id: ctx.attempt_id().clone(),
                    semantic_type_id: ctx.descriptor().output_semantic_type_id.clone(),
                    schema_id: ctx.descriptor().output_schema_id.clone(),
                    value_lineage: ctx.output_cell().value_lineage.clone(),
                    context: ctx.output_cell().context.clone(),
                    artifact_id: output_artifact,
                    content_digest: self.output_digest.clone(),
                    evidence_hash,
                    producer_state_kind: Some(ctx.node().state_kind.clone()),
                    producer_state_version: Some(ctx.node().state_version.clone()),
                })],
            ))
        })
    }
}

pub(super) struct ContextSourceRunner {
    stage: ContextStage,
    payload_context: Option<spec::CellContextSpec>,
    extractor: TypedContextOutputExtractor<RuntimeContextOutput>,
}

impl ContextSourceRunner {
    pub(super) fn new() -> Self {
        Self {
            stage: runtime_context_stage(),
            payload_context: None,
            extractor: TypedContextOutputExtractor::new(),
        }
    }

    pub(super) fn with_stage(stage: ContextStage) -> Self {
        Self {
            stage,
            ..Self::new()
        }
    }

    pub(super) fn with_payload_context(payload_context: spec::CellContextSpec) -> Self {
        Self {
            payload_context: Some(payload_context),
            ..Self::new()
        }
    }
}

impl ErasedNodeRunner for ContextSourceRunner {
    fn context_output_extractor(&self) -> Option<&dyn ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let context = ctx.certified_context::<RuntimeContractContext>()?;
            assert_eq!(context.value().network, "primary");
            let value = RuntimeContextOutput {
                amount: 6,
                context_ref: context.context_ref().clone(),
                resource_kind: runtime_context_resource_kind(),
                stage: self.stage.clone(),
            };
            let mut builder = RunnerOutputBuilder::new(&ctx);
            builder.state_output(&value)?;
            let mut output = builder.finish();
            if let Some(payload_context) = &self.payload_context {
                for payload in output.payloads_mut() {
                    if let RunnerEventPayload::CellProduced(produced) = payload {
                        produced.context = payload_context.clone();
                    }
                }
            }
            Ok(output)
        })
    }
}

pub(super) struct ContextConsumerRunner;

impl ErasedNodeRunner for ContextConsumerRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let context = ctx.certified_context::<RuntimeContractContext>()?;
            assert_eq!(context.value().network, "primary");
            let mut builder = RunnerOutputBuilder::new(&ctx);
            builder.state_output(&CertifierValue { amount: 30 })?;
            Ok(builder.finish())
        })
    }
}

pub(super) struct BlockingRunner;

impl ErasedNodeRunner for BlockingRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            Err(RuntimeError::Blocked(format!(
                "node {} is failed manually in this fixture",
                ctx.node().node_id
            )))
        })
    }
}

pub(super) struct ErrorRunner {
    pub(super) error: RuntimeError,
}

impl ErasedNodeRunner for ErrorRunner {
    fn run_erased<'a>(&'a self, _ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { Err(self.error.clone()) })
    }
}

pub(super) fn runner_kit_skipped_cell() -> MaterializedInputNode {
    MaterializedInputNode::Cell(Box::new(MaterializedCell {
        cell_id: CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D8),
        schema_id: fixture_value_schema_id(),
        semantic_type_id: fixture_value_semantic_id(),
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0x78),
        },
        context: spec::CellContextSpec::no_context(),
        terminal: MaterializedCellTerminal::Skipped {
            skip_reason: events::SkipReason {
                code: events::ErrorCode::new("runner_kit_skip").expect("skip code"),
                safe_message: "input was skipped".to_owned(),
            },
        },
    }))
}
