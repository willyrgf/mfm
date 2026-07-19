use super::*;

#[derive(Clone)]
pub(super) enum TestSubmissionDecision {
    Observed,
    Unknown,
    NotSubmitted,
    Ambiguous,
}

#[derive(Clone)]
pub(super) struct TestSideEffectDriverCallbacks {
    cap_kind: CapabilityKind,
    cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
    submission_decision: TestSubmissionDecision,
    preparation_settled: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

impl TestSideEffectDriverCallbacks {
    pub(super) fn new(fixture: &Fixture) -> Self {
        Self {
            cap_kind: side_effect_capability_kind(),
            cap_version: side_effect_capability_version(),
            adapter_kind: fixture.adapter_kind.clone(),
            adapter_version: fixture.adapter_version.clone(),
            submission_decision: TestSubmissionDecision::Observed,
            preparation_settled: None,
        }
    }

    pub(super) fn with_submission_decision(mut self, decision: TestSubmissionDecision) -> Self {
        self.submission_decision = decision;
        self
    }

    pub(super) fn with_preparation_settlement(
        mut self,
        settled: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        self.preparation_settled = Some(settled);
        self
    }

    pub(super) fn authored_intent_for(
        &self,
        node_id: String,
        attempt_id: String,
    ) -> mfm_program::SideEffectIntent<FixtureSideEffectEvidence, FixtureSideEffectEvidence> {
        mfm_program::SideEffectIntent::new(
            fixture_side_effect_evidence(21, node_id.clone(), attempt_id.clone()),
            fixture_side_effect_evidence(34, node_id, attempt_id),
        )
    }
}

impl SideEffectAdapter for TestSideEffectDriverCallbacks {
    type Intent = FixtureSideEffectEvidence;
    type Idempotency = FixtureSideEffectEvidence;
    type PreparedInvocation = FixtureSideEffectEvidence;
    type Submission = FixtureSideEffectEvidence;
    type RecoveryEvidence = FixtureSideEffectEvidence;
    type Receipt = FixtureSideEffectEvidence;
    type Confirmation = FixtureSideEffectEvidence;
    type Output = FixtureOutputValue;

    fn capability_binding(&self) -> Result<RunnerCapabilityBinding> {
        Ok(RunnerCapabilityBinding {
            capability_kind: self.cap_kind.clone(),
            capability_version: self.cap_version.clone(),
            adapter_kind: self.adapter_kind.clone(),
            adapter_version: self.adapter_version.clone(),
        })
    }

    fn authored_intent<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
    ) -> SideEffectDriverFuture<'a, mfm_program::SideEffectIntent<Self::Intent, Self::Idempotency>>
    {
        let node_id = submit_node.node_id.as_str().to_owned();
        let attempt_id = match &ctx.node().framework {
            Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => ctx
                .projections()
                .side_effect_for_pair(ctx.run_id(), &verify.pair_id)
                .map(|projection| projection.intent.attempt_id.as_str().to_owned())
                .unwrap_or_else(|| ctx.attempt_id().as_str().to_owned()),
            _ => ctx.attempt_id().as_str().to_owned(),
        };
        Box::pin(async move { Ok(self.authored_intent_for(node_id, attempt_id)) })
    }

    fn prepare<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _intent: &'a Self::Intent,
        _idempotency: &'a Self::Idempotency,
    ) -> SideEffectDriverFuture<'a, SideEffectPreparedInvocation<Self::PreparedInvocation>> {
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        let settled = self.preparation_settled.as_ref().map(Arc::clone);
        Box::pin(async move {
            let prepared = fixture_side_effect_evidence(35, node_id, attempt_id);
            Ok(match settled {
                Some(settled) => SideEffectPreparedInvocation::with_settlement(
                    prepared,
                    RunnerOutputSettlement::on_appended(move || {
                        settled.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }),
                ),
                None => SideEffectPreparedInvocation::new(prepared),
            })
        })
    }

    fn load_prepared<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        prepared: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::PreparedInvocation> {
        let prepared_artifact_id = prepared.artifact_id.clone();
        Box::pin(async move {
            Ok(fixture_side_effect_evidence(
                35,
                prepared_artifact_id.as_str().to_owned(),
                "reconstructed".to_owned(),
            ))
        })
    }

    fn submit_prepared<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _prepared: Self::PreparedInvocation,
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectSubmissionDecision<Self::Submission, Self::RecoveryEvidence>,
    > {
        let decision = self.submission_decision.clone();
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(match decision {
                TestSubmissionDecision::Observed => SideEffectSubmissionDecision::Observed(
                    fixture_side_effect_evidence(55, node_id.clone(), attempt_id.clone()),
                ),
                TestSubmissionDecision::Unknown => SideEffectSubmissionDecision::Unknown(
                    fixture_side_effect_evidence(56, node_id.clone(), attempt_id.clone()),
                ),
                TestSubmissionDecision::NotSubmitted => SideEffectSubmissionDecision::NotSubmitted(
                    fixture_side_effect_evidence(57, node_id.clone(), attempt_id.clone()),
                ),
                TestSubmissionDecision::Ambiguous => SideEffectSubmissionDecision::Ambiguous {
                    ambiguity_code: events::AmbiguityCode::new("mfm_test_driver_ambiguous")
                        .expect("ambiguity code"),
                    evidence: fixture_side_effect_evidence(58, node_id, attempt_id),
                },
            })
        })
    }
    fn recover_unknown<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _prepared: Self::PreparedInvocation,
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectUnknownSubmissionDecision<Self::Submission, Self::RecoveryEvidence>,
    > {
        let decision = self.submission_decision.clone();
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(match decision {
                TestSubmissionDecision::Observed => SideEffectUnknownSubmissionDecision::Observed(
                    fixture_side_effect_evidence(55, node_id.clone(), attempt_id.clone()),
                ),
                TestSubmissionDecision::Unknown => {
                    SideEffectUnknownSubmissionDecision::StillUnknown
                }
                TestSubmissionDecision::NotSubmitted => {
                    SideEffectUnknownSubmissionDecision::NotSubmitted(fixture_side_effect_evidence(
                        57,
                        node_id.clone(),
                        attempt_id.clone(),
                    ))
                }
                TestSubmissionDecision::Ambiguous => {
                    SideEffectUnknownSubmissionDecision::Ambiguous {
                        ambiguity_code: events::AmbiguityCode::new("mfm_test_driver_ambiguous")
                            .expect("ambiguity code"),
                        evidence: fixture_side_effect_evidence(58, node_id, attempt_id),
                    }
                }
            })
        })
    }

    fn observe_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _prepared: &'a store::SideEffectArtifactProjection,
        _submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        let evidence = fixture_side_effect_evidence_for_ctx(ctx, 89);
        Box::pin(async move {
            Ok(SideEffectObservedEvidence::new(
                evidence,
                test_driver_replay_evidence(),
            ))
        })
    }

    fn observe_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _prepared: &'a store::SideEffectArtifactProjection,
        _submission: &'a store::SideEffectArtifactProjection,
        _receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        let evidence = fixture_side_effect_evidence_for_ctx(ctx, 144);
        Box::pin(async move {
            Ok(SideEffectObservedEvidence::new(
                evidence,
                test_driver_replay_evidence(),
            ))
        })
    }

    fn output_from_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submit_inputs: &'a MaterializedInputs,
        _prepared: &'a store::SideEffectArtifactProjection,
        _submission: &'a store::SideEffectArtifactProjection,
        _receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        let output =
            fixture_output_value(233, ctx.node().node_id.as_str(), ctx.attempt_id().as_str());
        Box::pin(async move { Ok(output) })
    }

    fn output_from_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submit_inputs: &'a MaterializedInputs,
        _prepared: &'a store::SideEffectArtifactProjection,
        _submission: &'a store::SideEffectArtifactProjection,
        _receipt: &'a store::SideEffectArtifactProjection,
        _confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        let output =
            fixture_output_value(233, ctx.node().node_id.as_str(), ctx.attempt_id().as_str());
        Box::pin(async move { Ok(output) })
    }
}

fn test_driver_replay_evidence() -> SideEffectReplayEvidence {
    SideEffectReplayEvidence::new(
        events::ReplayVerifierId::new("mfm.test.driver.verifier").expect("verifier"),
        None,
    )
}

pub(super) fn test_driver_resource_key_for_node(
    node: &spec::NodeSpec,
) -> Option<events::ResourceKeyEvidence> {
    let Some(spec::SideEffectContractSpec {
        resource_claim:
            spec::ResourceClaimSpec::Exclusive {
                namespace,
                key_schema,
            },
        ..
    }) = &node.side_effect
    else {
        return None;
    };
    Some(events::ResourceKeyEvidence {
        namespace: namespace.clone(),
        key_schema_id: key_schema.clone(),
        key: events::ResourceKey::new("mfm.test.driver.shared-resource").expect("resource key"),
    })
}
