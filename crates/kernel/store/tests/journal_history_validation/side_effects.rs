use super::*;

use mfm_capabilities::{ExternalMutationAuthorityRole, NoCaps, Pure};
use mfm_events::v1::side_effect;
use mfm_ids::{AdapterVersion, SideEffectPairId};
use mfm_program::{
    PureState, ResourceClaim, ResourceNamespace, SideEffectState, SideEffectVerificationSpec,
};

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.store.test.side_effect_history_outputs")]
struct SideEffectHistoryOutputs<'program, 'scope> {
    side_effect: mfm_program::Handle<'program, 'scope, HistoryValue>,
    unrelated: mfm_program::Handle<'program, 'scope, HistoryValue>,
}

struct HistoryMutationCapability;

impl CapabilitySpec for HistoryMutationCapability {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.store.test",
            "history_mutation",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x71; 32]),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.store.test.history_mutation.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.store.test.history_mutation"
    }
}

struct HistoryMutationState {
    config: HistoryConfig,
}

impl StateSpec for HistoryMutationState {
    type Config = HistoryConfig;
    type Context = NoContext;
    type Input = HistoryValue;
    type Output = HistoryValue;
    type Effect = mfm_capabilities::ApplySideEffect;
    type Caps = (HistoryMutationCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.store.test",
            "history_mutation_state",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x72; 32]),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.store.test.history_mutation_state.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.store.test.history_mutation_state"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        Ok(vec![AdapterBindingSpec {
            adapter_kind: mutation_adapter_kind(),
            adapter_version: mutation_adapter_version(),
        }])
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl SideEffectState for HistoryMutationState {
    type Intent = HistoryValue;
    type IdempotencyInput = HistoryValue;
    type PreparedInvocation = HistoryValue;
    type Submission = HistoryValue;
    type RecoveryEvidence = HistoryValue;
    type Receipt = HistoryValue;
    type Confirmation = HistoryValue;

    fn intent(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<mfm_program::SideEffectIntent<Self::Intent, Self::IdempotencyInput>> {
        let intent = HistoryValue {
            amount: input.amount * self.config.multiplier,
        };
        Ok(mfm_program::SideEffectIntent::new(intent.clone(), intent))
    }

    fn output_from_receipt(
        &self,
        _input: &Self::Input,
        _prepared: &Self::PreparedInvocation,
        _submission: &Self::Submission,
        receipt: &Self::Receipt,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(receipt.clone())
    }

    fn output_from_confirmation(
        &self,
        _input: &Self::Input,
        _prepared: &Self::PreparedInvocation,
        _submission: &Self::Submission,
        _receipt: &Self::Receipt,
        confirmation: &Self::Confirmation,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(confirmation.clone())
    }
}

struct UnrelatedHistoryState {
    config: HistoryConfig,
}

impl StateSpec for UnrelatedHistoryState {
    type Config = HistoryConfig;
    type Context = NoContext;
    type Input = HistoryValue;
    type Output = HistoryValue;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.store.test",
            "unrelated_history_state",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x73; 32]),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.store.test.unrelated_history_state.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.store.test.unrelated_history_state"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for UnrelatedHistoryState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(HistoryValue {
            amount: input.amount * self.config.multiplier,
        })
    }
}

#[derive(Clone, Copy)]
enum HistorySaga {
    Fail,
    Manual,
}

struct SideEffectHistoryFixture {
    certified: mfm_certify::CertifiedTypedSpec,
    run_id: RunId,
    submit: spec::NodeSpec,
    unrelated: spec::NodeSpec,
    records: Vec<KernelEventEnvelope>,
    objects: ArtifactByteAuthorityMap,
}

fn certified_side_effect_spec(
    saga: HistorySaga,
    verification: SideEffectVerificationSpec,
) -> mfm_certify::CertifiedTypedSpec {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<HistoryMutationState>()
        .expect("mutation state registration");
    states
        .register::<UnrelatedHistoryState>()
        .expect("unrelated state registration");
    let manual = matches!(saga, HistorySaga::Manual).then(manual_saga_policy);
    let manual_spec = manual.as_ref().map(|(_, spec)| spec.clone());
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root scope"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            match saga {
                HistorySaga::Fail => {
                    root.set_saga_policy(mfm_program::SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
                }
                HistorySaga::Manual => {
                    root.set_saga_policy(manual.as_ref().expect("manual policy").0.clone())?;
                }
            }
            let input = root.seed(
                mfm_program::SeedKey::new("initial")?,
                CanonicalSeed::from_value(&HistoryValue { amount: 7 })?,
            )?;
            let side_effect = root.scope().side_effect::<HistoryMutationState, _>(
                StateKey::new("history-mutation")?,
                NoContext,
                HistoryConfig { multiplier: 2 },
                input.clone(),
                ResourceClaim::exclusive(resource_namespace(), resource_key_schema()),
                verification,
            )?;
            let unrelated = root.scope().state::<UnrelatedHistoryState, _>(
                StateKey::new("unrelated-history")?,
                NoContext,
                HistoryConfig { multiplier: 2 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &SideEffectHistoryOutputs {
                    side_effect: side_effect.into_handle(),
                    unrelated,
                },
            )
        },
    )
    .expect("side-effect history draft");
    let mut registry =
        mfm_certify::CertificationRegistry::from_program_draft(&draft).expect("registry");
    if let Some(manual) = manual_spec {
        registry
            .register_schema_role(
                manual.evidence_schema.clone(),
                mfm_certify::CertifiedSchemaRole::ManualResolutionEvidence,
            )
            .expect("manual evidence role");
        registry
            .register_manual_authorization_verifier(manual.authorization.verifier_id.clone())
            .expect("manual verifier");
        registry
            .register_operator_authority_snapshot(manual.authorization.authority.clone())
            .expect("manual authority");
    }
    let lowered = mfm_certify::lower_program_draft(&draft).expect("lowered side-effect history");
    mfm_certify::certify_typed_spec(lowered, &registry).expect("certified side-effect history")
}

fn manual_saga_policy() -> (
    mfm_program::SideEffectSagaPolicy,
    spec::ManualResolutionEvidenceSpec,
) {
    let operator = mfm_program::OperatorAuthorityMemberSpec {
        operator_id: mfm_program::OperatorId::new("mfm.store.test.manual.operator")
            .expect("operator id"),
        public_identity: mfm_program::OperatorPublicIdentity::new(
            "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
        )
        .expect("operator identity"),
    };
    let authority = mfm_program::OperatorAuthoritySnapshotDraft::new(
        mfm_program::OperatorAuthorityId::new("mfm.store.test.manual.authority")
            .expect("authority id"),
        mfm_program::NonEmptyUniqueOperators::new(operator, Vec::new())
            .expect("operator authority"),
    );
    let authorization = mfm_program::ManualAuthorizationDraft::threshold(
        mfm_program::ManualAuthorizationVerifierId::new("mfm.store.test.manual.verifier")
            .expect("verifier id"),
        mfm_program::ManualSigningSchemeSpec::new("mfm.manual_resolution.digest_signature.v1")
            .expect("signing scheme"),
        authority,
        mfm_program::ThresholdQuorum::new(1).expect("quorum"),
    )
    .expect("manual authorization");
    let manual =
        mfm_program::ManualResolutionPolicyDraft::new(manual_evidence_schema(), authorization);
    let manual_spec = manual.to_spec();
    (
        mfm_program::SideEffectSagaPolicy::ManualResolution {
            manual: Box::new(manual),
        },
        manual_spec,
    )
}

fn side_effect_fixture(
    saga: HistorySaga,
    verification: SideEffectVerificationSpec,
    terminal: SideEffectTerminal,
) -> SideEffectHistoryFixture {
    let prefix_certified = matches!(terminal, SideEffectTerminal::ManualResolution)
        .then(|| certified_side_effect_spec(saga, verification.clone()));
    let certified = certified_side_effect_spec(saga, verification);
    let spec = &certified.envelope().spec;
    let spec_hash = certified.spec_hash().clone();
    let submit = spec
        .nodes
        .iter()
        .find(|node| node.side_effect.is_some() && node.framework.is_none())
        .cloned()
        .expect("side-effect submit node");
    let (verify, pair_id) = spec
        .nodes
        .iter()
        .find_map(|node| match &node.framework {
            Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => {
                Some((node.clone(), verify.pair_id.clone()))
            }
            _ => None,
        })
        .expect("side-effect verify node");
    let unrelated = spec
        .nodes
        .iter()
        .find(|node| {
            node.framework.is_none()
                && node.state_kind == UnrelatedHistoryState::kind().expect("unrelated state kind")
        })
        .cloned()
        .expect("unrelated node");
    let submit_cell = spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == submit.output_cell)
        .cloned()
        .expect("submit output cell");
    let verify_cell = spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == verify.output_cell)
        .cloned()
        .expect("verify output cell");
    let submit_attempt = attempt_id(0x74);
    let verify_attempt = attempt_id(0x75);
    let unrelated_attempt = attempt_id(0x76);
    let ledger_key = events::SideEffectLedgerKey::new("history-ledger").expect("ledger key");
    let purpose = events::SideEffectLedgerPurpose::Forward;
    let resource_key = resource_key();
    let claim_id = events::ResourceLaneClaimId::new("history-lane-claim").expect("claim id");
    let (admitted, mut objects, run_id) = admitted_side_effect_run(&certified);
    let mut records = vec![persisted_record(
        &run_id,
        1,
        1,
        0,
        "side-effect-admission",
        KernelEventPayload::RunAdmitted(Box::new(admitted)),
    )];
    records.push(attempt_started(
        &run_id,
        2,
        &spec_hash,
        &submit,
        &submit_attempt,
        "submit-start",
    ));

    let intent = side_effect_object(
        br#"{"amount":14}"#,
        ArtifactRole::SideEffectIntent,
        schema_id("mfm.store.test.side_effect_intent", 0x77),
        Some(submit.node_id.clone()),
    );
    let prepared = side_effect_object(
        br#"{"amount":14}"#,
        ArtifactRole::PreparedInvocation,
        schema_id("mfm.store.test.prepared_invocation", 0x78),
        Some(submit.node_id.clone()),
    );
    let submission = side_effect_object(
        br#"{"amount":14}"#,
        ArtifactRole::Submission,
        schema_id("mfm.store.test.submission", 0x79),
        Some(submit.node_id.clone()),
    );
    for (bytes, evidence) in [&intent, &prepared, &submission] {
        objects.insert(object_key(evidence), (bytes.clone(), evidence.clone()));
    }

    let submit_progress = vec![
        KernelEventPayload::SideEffectIntentPersisted(side_effect::IntentPersisted {
            spec_hash: spec_hash.clone(),
            node_id: submit.node_id.clone(),
            scope_id: submit.scope_id.clone(),
            attempt_id: submit_attempt.clone(),
            ledger_key: ledger_key.clone(),
            ledger_purpose: purpose.clone(),
            pair_id: pair_id.clone(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            intent_schema_id: intent.1.schema_id.clone().expect("intent schema"),
            intent_hash: intent.1.digest.clone(),
            intent_artifact_id: intent.1.artifact_id.clone(),
            intent_artifact_evidence_hash: intent.1.evidence_hash().expect("intent evidence"),
            idempotency_input_schema_id: schema_id("mfm.store.test.idempotency_input", 0x7a),
            idempotency_input_hash: fixed_content_digest_for_test(0x7b),
            idempotency_key: events::IdempotencyKeyRef::new("history-idempotency")
                .expect("idempotency key"),
            capability_kind: HistoryMutationCapability::kind().expect("capability kind"),
            capability_version: HistoryMutationCapability::version().expect("capability version"),
            adapter_kind: mutation_adapter_kind(),
            adapter_version: mutation_adapter_version(),
        }),
        KernelEventPayload::SideEffectClaimed(side_effect::Claimed {
            spec_hash: spec_hash.clone(),
            node_id: submit.node_id.clone(),
            attempt_id: submit_attempt.clone(),
            ledger_key: ledger_key.clone(),
            ledger_purpose: purpose.clone(),
            pair_id: pair_id.clone(),
            pair_role: events::SideEffectPairRole::Submit,
            claim_owner: runner_invocation_id(),
            invocation_epoch: 1,
            claim_generation: 1,
            claim_fencing_token: claim_fencing_token(),
        }),
        KernelEventPayload::ResourceLaneClaimed(events::ResourceLaneClaimed {
            spec_hash: spec_hash.clone(),
            node_id: submit.node_id.clone(),
            attempt_id: submit_attempt.clone(),
            ledger_key: ledger_key.clone(),
            ledger_purpose: purpose.clone(),
            pair_id: pair_id.clone(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            resource_key: resource_key.clone(),
            requirement_digest: fixed_content_digest_for_test(0x7c),
            resolved_by_capability_impl: events::RunnerFactoryId::new(
                "mfm.store.test.history.runner",
            )
            .expect("runner factory"),
            claim_id: claim_id.clone(),
            claim_fencing_token: 7,
            lane_transition_seq: 1,
        }),
        KernelEventPayload::SideEffectInvocationPrepared(side_effect::InvocationPrepared {
            spec_hash: spec_hash.clone(),
            node_id: submit.node_id.clone(),
            attempt_id: submit_attempt.clone(),
            ledger_key: ledger_key.clone(),
            ledger_purpose: purpose.clone(),
            pair_id: pair_id.clone(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            claim_generation: 1,
            claim_fencing_token: claim_fencing_token(),
            resource_key: Some(resource_key),
            prepared_schema_id: prepared.1.schema_id.clone().expect("prepared schema"),
            prepared_artifact_id: prepared.1.artifact_id.clone(),
            prepared_hash: prepared.1.digest.clone(),
            prepared_artifact_evidence_hash: prepared.1.evidence_hash().expect("prepared evidence"),
        }),
        KernelEventPayload::SideEffectInvocationStarted(side_effect::InvocationStarted {
            spec_hash: spec_hash.clone(),
            node_id: submit.node_id.clone(),
            attempt_id: submit_attempt.clone(),
            ledger_key: ledger_key.clone(),
            ledger_purpose: purpose.clone(),
            pair_id: pair_id.clone(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            claim_owner: runner_invocation_id(),
            claim_generation: 1,
            claim_fencing_token: claim_fencing_token(),
        }),
        KernelEventPayload::SideEffectSubmissionObserved(side_effect::SubmissionObserved {
            spec_hash: spec_hash.clone(),
            node_id: submit.node_id.clone(),
            attempt_id: submit_attempt.clone(),
            ledger_key: ledger_key.clone(),
            ledger_purpose: purpose.clone(),
            pair_id: pair_id.clone(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            submission_schema_id: submission.1.schema_id.clone().expect("submission schema"),
            submission_hash: submission.1.digest.clone(),
            submission_artifact_id: submission.1.artifact_id.clone(),
            submission_artifact_evidence_hash: submission
                .1
                .evidence_hash()
                .expect("submission evidence"),
        }),
    ];
    for (index, payload) in submit_progress.into_iter().enumerate() {
        records.push(persisted_record(
            &run_id,
            3 + index as u64,
            3 + index as u64,
            0,
            &format!("submit-progress-{index}"),
            payload,
        ));
    }

    let submit_terminal_seq = 9;
    records.push(persisted_record(
        &run_id,
        submit_terminal_seq,
        submit_terminal_seq,
        0,
        "submit-terminal",
        KernelEventPayload::CellSkipped(events::CellSkipped {
            spec_hash: spec_hash.clone(),
            node_id: submit.node_id.clone(),
            cell_id: submit_cell.cell_id.clone(),
            scope_id: submit_cell.scope_id.clone(),
            attempt_id: submit_attempt.clone(),
            semantic_type_id: submit_cell.semantic_type_id.clone(),
            schema_id: submit_cell.schema_id.clone(),
            value_lineage: submit_cell.value_lineage.clone(),
            context: submit_cell.context.clone(),
            skip_reason: events::SkipReason {
                code: events::ErrorCode::new("submission_boundary").expect("skip code"),
                safe_message: "submission boundary reached".to_owned(),
            },
        }),
    ));
    records.push(persisted_record(
        &run_id,
        submit_terminal_seq,
        submit_terminal_seq,
        1,
        "submit-terminal",
        KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
            spec_hash: spec_hash.clone(),
            node_id: submit.node_id.clone(),
            attempt_id: submit_attempt,
            output_cell_id: submit_cell.cell_id.clone(),
        }),
    ));
    records.push(attempt_started(
        &run_id,
        10,
        &spec_hash,
        &verify,
        &verify_attempt,
        "verify-start",
    ));
    records.push(attempt_started(
        &run_id,
        11,
        &spec_hash,
        &unrelated,
        &unrelated_attempt,
        "unrelated-start",
    ));

    match terminal {
        SideEffectTerminal::Receipt => {
            append_receipt_terminal(
                &mut records,
                &mut objects,
                &run_id,
                &spec_hash,
                &verify,
                &verify_cell,
                &verify_attempt,
                &ledger_key,
                &purpose,
                &pair_id,
                &claim_id,
            );
            records.push(attempt_interrupted(
                &run_id,
                13,
                &spec_hash,
                &unrelated,
                &unrelated_attempt,
                "unrelated-interrupted",
            ));
        }
        SideEffectTerminal::Ambiguous => {
            append_ambiguous_terminal(
                &mut records,
                &mut objects,
                &run_id,
                &spec_hash,
                &verify,
                &verify_attempt,
                &ledger_key,
                &purpose,
                &pair_id,
            );
            records.push(attempt_interrupted(
                &run_id,
                13,
                &spec_hash,
                &unrelated,
                &unrelated_attempt,
                "unrelated-interrupted",
            ));
        }
        SideEffectTerminal::ManualResolution => {
            append_ambiguous_terminal(
                &mut records,
                &mut objects,
                &run_id,
                &spec_hash,
                &verify,
                &verify_attempt,
                &ledger_key,
                &purpose,
                &pair_id,
            );
            records.push(attempt_interrupted(
                &run_id,
                13,
                &spec_hash,
                &unrelated,
                &unrelated_attempt,
                "unrelated-interrupted",
            ));
            let prefix_view = load_journal(&run_id, records.clone(), objects.clone())
                .expect("manual prefix journal")
                .verify(prefix_certified.expect("manual prefix certified spec"))
                .expect("verified manual prefix");
            let prefix = mfm_store::v1::current_lifecycle::read(&prefix_view)
                .manual_resolution_prefix_authority()
                .expect("manual prefix authority");
            append_manual_resolution(
                &mut records,
                &mut objects,
                &run_id,
                &spec_hash,
                &ledger_key,
                &purpose,
                &pair_id,
                &claim_id,
                &prefix,
            );
        }
    }

    SideEffectHistoryFixture {
        certified,
        run_id,
        submit,
        unrelated,
        records,
        objects,
    }
}

#[derive(Clone, Copy)]
enum SideEffectTerminal {
    Receipt,
    Ambiguous,
    ManualResolution,
}

#[allow(clippy::too_many_arguments)]
fn append_receipt_terminal(
    records: &mut Vec<KernelEventEnvelope>,
    objects: &mut ArtifactByteAuthorityMap,
    run_id: &RunId,
    spec_hash: &mfm_ids::SpecHash,
    verify: &spec::NodeSpec,
    verify_cell: &spec::CellSpec,
    verify_attempt: &AttemptId,
    ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
    pair_id: &SideEffectPairId,
    claim_id: &events::ResourceLaneClaimId,
) {
    let receipt = side_effect_object(
        br#"{"amount":14}"#,
        ArtifactRole::Receipt,
        schema_id("mfm.store.test.receipt", 0x7d),
        Some(verify.node_id.clone()),
    );
    let mut output = side_effect_object(
        br#"{"amount":14}"#,
        ArtifactRole::StateOutput,
        verify_cell.schema_id.clone(),
        Some(verify.node_id.clone()),
    );
    output.1.semantic_type_id = Some(verify_cell.semantic_type_id.clone());
    objects.insert(
        object_key(&receipt.1),
        (receipt.0.clone(), receipt.1.clone()),
    );
    objects.insert(object_key(&output.1), (output.0, output.1.clone()));
    let payloads = vec![
        resource_lane_released(
            spec_hash,
            ledger_key,
            purpose,
            pair_id,
            claim_id,
            events::ResourceLaneReleaseAuthority::VerifyTerminal,
        ),
        KernelEventPayload::SideEffectReceiptObserved(side_effect::ReceiptObserved {
            spec_hash: spec_hash.clone(),
            node_id: verify.node_id.clone(),
            attempt_id: verify_attempt.clone(),
            ledger_key: ledger_key.clone(),
            ledger_purpose: purpose.clone(),
            pair_id: pair_id.clone(),
            pair_role: events::SideEffectPairRole::Verify,
            invocation_epoch: 1,
            receipt_schema_id: receipt.1.schema_id.clone().expect("receipt schema"),
            receipt_hash: receipt.1.digest.clone(),
            receipt_artifact_id: receipt.1.artifact_id.clone(),
            receipt_artifact_evidence_hash: receipt.1.evidence_hash().expect("receipt evidence"),
            replay_verifier_id: events::ReplayVerifierId::new("history-replay-verifier")
                .expect("replay verifier"),
            resource_touched_set: None,
        }),
        KernelEventPayload::CellProduced(events::CellProduced {
            spec_hash: spec_hash.clone(),
            node_id: verify.node_id.clone(),
            cell_id: verify_cell.cell_id.clone(),
            scope_id: verify_cell.scope_id.clone(),
            attempt_id: verify_attempt.clone(),
            semantic_type_id: verify_cell.semantic_type_id.clone(),
            schema_id: verify_cell.schema_id.clone(),
            value_lineage: verify_cell.value_lineage.clone(),
            context: verify_cell.context.clone(),
            artifact_id: output.1.artifact_id.clone(),
            content_digest: output.1.digest.clone(),
            evidence_hash: output.1.evidence_hash().expect("output evidence"),
            producer_state_kind: Some(verify.state_kind.clone()),
            producer_state_version: Some(verify.state_version.clone()),
        }),
        KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
            spec_hash: spec_hash.clone(),
            node_id: verify.node_id.clone(),
            attempt_id: verify_attempt.clone(),
            output_cell_id: verify_cell.cell_id.clone(),
        }),
    ];
    append_batch(records, run_id, 12, "verify-terminal", payloads);
}

#[allow(clippy::too_many_arguments)]
fn append_ambiguous_terminal(
    records: &mut Vec<KernelEventEnvelope>,
    objects: &mut ArtifactByteAuthorityMap,
    run_id: &RunId,
    spec_hash: &mfm_ids::SpecHash,
    verify: &spec::NodeSpec,
    verify_attempt: &AttemptId,
    ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
    pair_id: &SideEffectPairId,
) {
    let ambiguity = side_effect_object(
        br#"{"ambiguous":true}"#,
        ArtifactRole::AmbiguityEvidence,
        schema_id("mfm.store.test.ambiguity", 0x7e),
        Some(verify.node_id.clone()),
    );
    objects.insert(object_key(&ambiguity.1), (ambiguity.0, ambiguity.1.clone()));
    append_batch(
        records,
        run_id,
        12,
        "verify-ambiguous",
        vec![
            KernelEventPayload::SideEffectAmbiguous(side_effect::Ambiguous {
                spec_hash: spec_hash.clone(),
                node_id: verify.node_id.clone(),
                attempt_id: verify_attempt.clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role: events::SideEffectPairRole::Verify,
                invocation_epoch: 1,
                ambiguity_code: events::AmbiguityCode::new("history_ambiguous")
                    .expect("ambiguity code"),
                evidence_schema_id: ambiguity.1.schema_id.clone().expect("ambiguity schema"),
                evidence_hash: ambiguity.1.digest.clone(),
                evidence_artifact_id: ambiguity.1.artifact_id.clone(),
                evidence_artifact_evidence_hash: ambiguity
                    .1
                    .evidence_hash()
                    .expect("ambiguity evidence"),
            }),
            KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                spec_hash: spec_hash.clone(),
                node_id: verify.node_id.clone(),
                attempt_id: verify_attempt.clone(),
                retryable: false,
                error: side_effect_error(),
            }),
        ],
    );
}

#[allow(clippy::too_many_arguments)]
fn append_manual_resolution(
    records: &mut Vec<KernelEventEnvelope>,
    objects: &mut ArtifactByteAuthorityMap,
    run_id: &RunId,
    spec_hash: &mfm_ids::SpecHash,
    ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
    pair_id: &SideEffectPairId,
    claim_id: &events::ResourceLaneClaimId,
    prefix: &mfm_manual_auth::ManualResolutionPrefixAuthority,
) {
    let evidence = side_effect_object(
        br#"{"manual":"evidence"}"#,
        ArtifactRole::ManualResolutionEvidence,
        manual_evidence_schema(),
        None,
    );
    let outcome = events::ManualResolutionOutcome::FailWithoutAcdcClaim;
    let evidence_ref = mfm_manual_auth::ManualResolutionEvidenceRef {
        schema_id: evidence
            .1
            .schema_id
            .clone()
            .expect("manual evidence schema"),
        content_hash: evidence.1.digest.clone(),
        artifact_id: evidence.1.artifact_id.clone(),
    };
    let proof = signed_manual_proof(prefix, outcome, evidence_ref);
    let authorization = side_effect_object(
        proof
            .canonical_json()
            .expect("canonical manual authorization")
            .as_bytes(),
        ArtifactRole::ManualResolutionAuthorization,
        mfm_manual_auth::manual_authorization_proof_schema_id()
            .expect("manual authorization schema"),
        None,
    );
    for (bytes, artifact) in [&evidence, &authorization] {
        objects.insert(object_key(artifact), (bytes.clone(), artifact.clone()));
    }
    append_batch(
        records,
        run_id,
        14,
        "manual-resolution",
        vec![
            resource_lane_released(
                spec_hash,
                ledger_key,
                purpose,
                pair_id,
                claim_id,
                events::ResourceLaneReleaseAuthority::ManualResolution,
            ),
            KernelEventPayload::ManualResolutionRecorded(events::ManualResolutionRecorded {
                run_id: run_id.clone(),
                spec_hash: spec_hash.clone(),
                outcome,
                evidence_schema_id: manual_evidence_schema(),
                evidence_hash: evidence.1.digest.clone(),
                evidence_artifact_id: evidence.1.artifact_id.clone(),
                evidence_artifact_evidence_hash: evidence
                    .1
                    .evidence_hash()
                    .expect("manual evidence identity"),
                authorization_schema_id: authorization
                    .1
                    .schema_id
                    .clone()
                    .expect("authorization schema"),
                authorization_hash: authorization.1.digest.clone(),
                authorization_artifact_id: authorization.1.artifact_id.clone(),
                authorization_artifact_evidence_hash: authorization
                    .1
                    .evidence_hash()
                    .expect("manual authorization identity"),
                note: Some(
                    events::ManualResolutionNote::new("history manual resolution")
                        .expect("manual note"),
                ),
            }),
        ],
    );
}

fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
    let mut key_bytes = [0_u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test manual key");
    k256::ecdsa::SigningKey::from(&secret_key)
}

fn sign_manual_claim_digest(signing_key: &k256::ecdsa::SigningKey, digest: &[u8; 32]) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}

fn signed_manual_proof(
    prefix: &mfm_manual_auth::ManualResolutionPrefixAuthority,
    outcome: events::ManualResolutionOutcome,
    evidence: mfm_manual_auth::ManualResolutionEvidenceRef,
) -> mfm_manual_auth::ManualResolutionAuthorizationProof {
    let claim = prefix
        .authorization_claim(outcome, evidence)
        .expect("manual authorization claim");
    let policy = &prefix.manual_policy().authorization;
    let operator = policy.authority.operators[0].clone();
    let signature = sign_manual_claim_digest(
        &test_manual_signing_key(),
        claim
            .digest()
            .expect("manual claim digest")
            .digest()
            .as_bytes(),
    );
    mfm_manual_auth::ManualResolutionAuthorizationProof {
        verifier_id: policy.verifier_id.clone(),
        signing_scheme: policy.signing_scheme.clone(),
        claim,
        signatures: vec![mfm_manual_auth::ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: mfm_manual_auth::ManualAuthorizationSignatureBytes::new(signature)
                .expect("manual signature"),
        }],
    }
}

fn manual_resolution_index(fixture: &SideEffectHistoryFixture) -> usize {
    fixture
        .records
        .iter()
        .position(|record| {
            matches!(
                record.payload(),
                KernelEventPayload::ManualResolutionRecorded(_)
            )
        })
        .expect("manual resolution record")
}

fn manual_resolution_payload(
    fixture: &SideEffectHistoryFixture,
) -> events::ManualResolutionRecorded {
    match fixture.records[manual_resolution_index(fixture)]
        .payload()
        .clone()
    {
        KernelEventPayload::ManualResolutionRecorded(payload) => payload,
        _ => unreachable!("located manual resolution"),
    }
}

fn manual_evidence_ref_from_fixture(
    fixture: &SideEffectHistoryFixture,
) -> mfm_manual_auth::ManualResolutionEvidenceRef {
    let manual = manual_resolution_payload(fixture);
    mfm_manual_auth::ManualResolutionEvidenceRef {
        schema_id: manual.evidence_schema_id,
        content_hash: manual.evidence_hash,
        artifact_id: manual.evidence_artifact_id,
    }
}

fn manual_authorization_proof(
    fixture: &SideEffectHistoryFixture,
) -> mfm_manual_auth::ManualResolutionAuthorizationProof {
    let manual = manual_resolution_payload(fixture);
    let bytes = &fixture
        .objects
        .get(&(
            manual.authorization_artifact_id,
            manual.authorization_artifact_evidence_hash,
        ))
        .expect("manual authorization object")
        .0;
    mfm_manual_auth::ManualResolutionAuthorizationProof::from_json_slice(bytes)
        .expect("canonical manual authorization proof")
}

fn replace_manual_authorization_proof(
    fixture: &mut SideEffectHistoryFixture,
    proof: &mfm_manual_auth::ManualResolutionAuthorizationProof,
) {
    replace_manual_authorization_bytes(
        fixture,
        proof
            .canonical_json()
            .expect("canonical manual authorization proof")
            .to_vec(),
    );
}

fn replace_manual_authorization_bytes(fixture: &mut SideEffectHistoryFixture, bytes: Vec<u8>) {
    let index = manual_resolution_index(fixture);
    let mut manual = manual_resolution_payload(fixture);
    fixture.objects.remove(&(
        manual.authorization_artifact_id.clone(),
        manual.authorization_artifact_evidence_hash.clone(),
    ));
    let evidence = exact_artifact(
        &bytes,
        ArtifactRole::ManualResolutionAuthorization,
        mfm_manual_auth::manual_authorization_proof_schema_id()
            .expect("manual authorization schema"),
        MediaType::new("application/json").expect("JSON media type"),
        None,
        None,
    );
    manual.authorization_schema_id = evidence
        .schema_id
        .clone()
        .expect("manual authorization schema");
    manual.authorization_hash = evidence.digest.clone();
    manual.authorization_artifact_id = evidence.artifact_id.clone();
    manual.authorization_artifact_evidence_hash = evidence
        .evidence_hash()
        .expect("manual authorization evidence");
    fixture
        .objects
        .insert(object_key(&evidence), (bytes, evidence));
    fixture.records[index] = replace_record_payload(
        &fixture.records[index],
        KernelEventPayload::ManualResolutionRecorded(manual),
    );
}

fn event_artifact_evidence(evidence: &ArtifactEvidenceRef) -> events::ArtifactEvidenceRef {
    events::ArtifactEvidenceRef {
        artifact_id: evidence.artifact_id.clone(),
        role: evidence.artifact_role,
        schema_id: evidence.schema_id.clone().expect("event artifact schema"),
        semantic_type_id: evidence.semantic_type_id.clone(),
        content_digest: evidence.digest.clone(),
        evidence_hash: evidence.evidence_hash().expect("event artifact evidence"),
        byte_len: evidence.byte_len,
        media_type: evidence.media_type.clone(),
    }
}

fn resource_lane_released(
    spec_hash: &mfm_ids::SpecHash,
    ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
    pair_id: &SideEffectPairId,
    claim_id: &events::ResourceLaneClaimId,
    authority: events::ResourceLaneReleaseAuthority,
) -> KernelEventPayload {
    KernelEventPayload::ResourceLaneReleased(events::ResourceLaneReleased {
        spec_hash: spec_hash.clone(),
        ledger_key: ledger_key.clone(),
        ledger_purpose: purpose.clone(),
        pair_id: pair_id.clone(),
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        claim_id: claim_id.clone(),
        release_id: events::ResourceLaneReleaseId::new("history-lane-release").expect("release id"),
        claim_fencing_token: 7,
        release_authority: authority,
        release_reason: events::ResourceLaneReleaseReason::new("history_terminal")
            .expect("release reason"),
        lane_transition_seq: 2,
    })
}

fn attempt_started(
    run_id: &RunId,
    sequence: u64,
    spec_hash: &mfm_ids::SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    commit_key: &str,
) -> KernelEventEnvelope {
    persisted_record(
        run_id,
        sequence,
        sequence,
        0,
        commit_key,
        KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: spec_hash.clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            attempt_no: 1,
            state_kind: node.state_kind.clone(),
            state_version: node.state_version.clone(),
        }),
    )
}

fn attempt_interrupted(
    run_id: &RunId,
    sequence: u64,
    spec_hash: &mfm_ids::SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    commit_key: &str,
) -> KernelEventEnvelope {
    persisted_record(
        run_id,
        sequence,
        sequence,
        0,
        commit_key,
        KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
            spec_hash: spec_hash.clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
        }),
    )
}

fn append_batch(
    records: &mut Vec<KernelEventEnvelope>,
    run_id: &RunId,
    sequence: u64,
    commit_key: &str,
    payloads: Vec<KernelEventPayload>,
) {
    for (ordinal, payload) in payloads.into_iter().enumerate() {
        records.push(persisted_record(
            run_id,
            sequence,
            sequence,
            ordinal as u32,
            commit_key,
            payload,
        ));
    }
}

fn admitted_side_effect_run(
    certified: &mfm_certify::CertifiedTypedSpec,
) -> (events::RunAdmitted, ArtifactByteAuthorityMap, RunId) {
    let spec = &certified.envelope().spec;
    let config_json = serde_json::to_string(&HistoryConfig { multiplier: 2 }).expect("config json");
    let state_config = PlainCanonicalJsonBytes::from_json_str(&config_json)
        .expect("canonical config")
        .to_vec();
    let config_objects = spec
        .config_refs
        .iter()
        .map(|config_ref| {
            let bytes = spec
                .nodes
                .iter()
                .find(|node| node.config_ref == *config_ref && node.framework.is_some())
                .map(|node| {
                    let framework = node.framework.as_ref().expect("framework node");
                    spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
                        .expect("framework config")
                        .to_vec()
                })
                .unwrap_or_else(|| state_config.clone());
            assert_eq!(
                ContentDigest::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    sha256_digest_bytes(&bytes)
                ),
                config_ref.digest
            );
            let evidence = ArtifactEvidenceRef {
                artifact_id: config_ref.artifact_id.clone(),
                digest: config_ref.digest.clone(),
                byte_len: config_ref.byte_len,
                media_type: config_ref.media_type.clone(),
                schema_id: Some(config_ref.schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: ArtifactRole::TypedConfig,
            };
            (bytes, evidence)
        })
        .collect::<Vec<_>>();
    let seed_spec = spec.seeds.first().cloned().expect("one side-effect seed");
    let seed = CanonicalSeed::from_value(&HistoryValue { amount: 7 }).expect("canonical seed");
    let seed_bytes = seed.canonical_json().as_bytes().to_vec();
    let seed_evidence = ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(
            seed.content_digest().algorithm(),
            *seed.content_digest().digest(),
        ),
        digest: seed.content_digest().clone(),
        byte_len: seed.byte_len() as u64,
        media_type: MediaType::new("application/json").expect("json media type"),
        schema_id: Some(seed_spec.schema_id.clone()),
        semantic_type_id: Some(seed_spec.semantic_type_id.clone()),
        producer_node_id: None,
        producer_seed_id: Some(seed_spec.seed_id.clone()),
        artifact_role: ArtifactRole::SeedInput,
    };
    let persisted = certified
        .to_persisted_parts()
        .expect("persisted certified authority");
    let spec_evidence = exact_artifact(
        persisted.spec_bytes(),
        ArtifactRole::TypedExecutionSpec,
        spec::typed_execution_spec_schema_id().expect("typed spec schema"),
        spec.media_type.clone(),
        None,
        None,
    );
    let certificate_evidence = exact_artifact(
        persisted.certificate_bytes(),
        ArtifactRole::TypedSpecCertificate,
        mfm_certify::typed_spec_certificate_schema_id().expect("certificate schema"),
        MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).expect("certificate media type"),
        None,
        None,
    );
    let identity_material =
        run_identity_material_for_test(certified.spec_hash().clone(), &"70".repeat(16));
    let run_id = identity_material.derive_run_id().expect("run id");
    let admitted = events::RunAdmitted {
        run_id: run_id.clone(),
        identity_material,
        entry_point: events::EntryPointLaunchEvidence::new(
            "mfm.store.test/side_effect_history@1",
            Vec::new(),
        )
        .expect("entry point"),
        spec_hash: certified.spec_hash().clone(),
        spec_artifact: run_artifact_ref_from_store_artifact_for_test(&spec_evidence),
        certificate_artifact: run_artifact_ref_from_store_artifact_for_test(&certificate_evidence),
        config_artifacts: config_objects
            .iter()
            .map(|(_, evidence)| run_artifact_ref_from_store_artifact_for_test(evidence))
            .collect(),
        fact_descriptor_artifacts: Vec::new(),
        spec_version: spec.spec_version.clone(),
        lowering_version: spec.lowering_version.clone(),
        public_output_schema_id: spec.public_outputs.public_schema_id.clone(),
        saga_policy_digest: spec.saga.saga_policy_digest().expect("saga digest"),
        descriptor_identities: spec.descriptor_identities.clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        capability_implementations: Vec::new(),
        admitted_binding_digest: fixed_content_digest_for_test(0x7f),
        canonicalizer_identity: spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            .clone(),
        seed_cells: vec![events::SeedCellRef {
            seed_id: seed_spec.seed_id,
            cell_id: seed_spec.cell_id,
            scope_id: seed_spec.scope_id,
            semantic_type_id: seed_spec.semantic_type_id.clone(),
            schema_id: seed_spec.schema_id.clone(),
            digest: seed_evidence.digest.clone(),
            seed_artifact: events::ArtifactEvidenceRef {
                artifact_id: seed_evidence.artifact_id.clone(),
                role: seed_evidence.artifact_role,
                schema_id: seed_spec.schema_id,
                semantic_type_id: Some(seed_spec.semantic_type_id),
                content_digest: seed_evidence.digest.clone(),
                evidence_hash: seed_evidence.evidence_hash().expect("seed evidence"),
                byte_len: seed_evidence.byte_len,
                media_type: seed_evidence.media_type.clone(),
            },
        }],
    };
    let mut objects = BTreeMap::from([
        (
            object_key(&spec_evidence),
            (persisted.spec_bytes().to_vec(), spec_evidence),
        ),
        (
            object_key(&certificate_evidence),
            (persisted.certificate_bytes().to_vec(), certificate_evidence),
        ),
        (object_key(&seed_evidence), (seed_bytes, seed_evidence)),
    ]);
    for (bytes, evidence) in config_objects {
        objects.insert(object_key(&evidence), (bytes, evidence));
    }
    (admitted, objects, run_id)
}

fn side_effect_object(
    bytes: &[u8],
    role: ArtifactRole,
    schema_id: SchemaId,
    producer_node_id: Option<NodeId>,
) -> (Vec<u8>, ArtifactEvidenceRef) {
    let json = std::str::from_utf8(bytes).expect("UTF-8 side-effect object");
    let bytes = PlainCanonicalJsonBytes::from_json_str(json)
        .expect("canonical side-effect object")
        .to_vec();
    let evidence = exact_artifact(
        &bytes,
        role,
        schema_id,
        MediaType::new("application/json").expect("json media type"),
        None,
        producer_node_id,
    );
    (bytes, evidence)
}

fn schema_id(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
    .expect("schema id")
}

fn mutation_adapter_kind() -> mfm_ids::AdapterKind {
    mfm_ids::AdapterKind::new(
        "mfm.store.test",
        "history_mutation_adapter",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x80; 32]),
    )
    .expect("adapter kind")
}

fn mutation_adapter_version() -> AdapterVersion {
    AdapterVersion::new("mfm.store.test.history_mutation_adapter.v1").expect("adapter version")
}

fn resource_namespace() -> ResourceNamespace {
    ResourceNamespace::new("mfm.store.test.history_resource").expect("resource namespace")
}

fn resource_key_schema() -> SchemaId {
    schema_id("mfm.store.test.history_resource_key", 0x81)
}

fn resource_key() -> events::ResourceKeyEvidence {
    events::ResourceKeyEvidence {
        namespace: resource_namespace(),
        key_schema_id: resource_key_schema(),
        key: events::ResourceKey::new("history-resource-1").expect("resource key"),
    }
}

fn manual_evidence_schema() -> SchemaId {
    schema_id("mfm.store.test.manual_evidence", 0x82)
}

fn runner_invocation_id() -> events::RunnerInvocationId {
    events::RunnerInvocationId::new("history-runner-invocation").expect("runner invocation")
}

fn claim_fencing_token() -> side_effect::ClaimFencingToken {
    side_effect::ClaimFencingToken::new("history-fencing-token").expect("fencing token")
}

fn side_effect_error() -> events::MfmErrorInfo {
    events::MfmErrorInfo {
        code: events::ErrorCode::new("history_ambiguous").expect("error code"),
        category: events::ErrorCategory::SideEffect,
        retryable: false,
        safe_message: "side effect is ambiguous".to_owned(),
        public_details: None,
        diagnostic_ref: None,
    }
}

fn verify_side_effect_fixture(
    fixture: SideEffectHistoryFixture,
) -> Result<VerifiedRunView, StoreError> {
    load_journal(&fixture.run_id, fixture.records, fixture.objects)?.verify(fixture.certified)
}

fn assert_side_effect_certified_history_rejects(fixture: SideEffectHistoryFixture, expected: &str) {
    let journal = load_journal(&fixture.run_id, fixture.records, fixture.objects)
        .expect("side-effect mutation remains structurally loadable");
    let error = journal
        .verify(fixture.certified)
        .expect_err("side-effect certified history must reject");
    assert!(
        matches!(
            &error,
            StoreError::PersistedEventMismatch {
                field: "certified_history",
                message,
            } if message.contains(expected)
        ),
        "unexpected error: {error:?}"
    );
}

fn assert_side_effect_current_projection_rejects(
    fixture: SideEffectHistoryFixture,
    expected: &str,
) {
    let journal = load_journal(&fixture.run_id, fixture.records, fixture.objects)
        .expect("mutation remains a structurally loadable committed journal");
    let error = journal
        .verify(fixture.certified)
        .expect_err("side-effect current projection must reject");
    assert!(
        matches!(
            &error,
            StoreError::ProjectionConflict { message, .. } if message.contains(expected)
        ),
        "unexpected error: {error:?}"
    );
}

#[test]
fn certified_history_accepts_exact_receipt_terminal_side_effect() {
    let fixture = side_effect_fixture(
        HistorySaga::Fail,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::Receipt,
    );
    let view = verify_side_effect_fixture(fixture).expect("valid receipt-terminal history");
    assert_eq!(view.current_run_sequence(), Some(13));
}

#[test]
fn certified_history_rejects_skipped_cell_context_mismatch() {
    let mut fixture = side_effect_fixture(
        HistorySaga::Fail,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::Receipt,
    );
    let index = fixture
        .records
        .iter()
        .position(|record| matches!(record.payload(), KernelEventPayload::CellSkipped(_)))
        .expect("submit skipped cell");
    let mut skipped = match fixture.records[index].payload().clone() {
        KernelEventPayload::CellSkipped(payload) => payload,
        _ => unreachable!("located skipped cell"),
    };
    skipped.context = mismatched_context(&fixture.submit);
    fixture.records[index] = replace_record_payload(
        &fixture.records[index],
        KernelEventPayload::CellSkipped(skipped),
    );
    assert_side_effect_certified_history_rejects(fixture, "does not match certified metadata");
}

#[test]
fn certified_history_rejects_ambiguity_emitted_outside_submit_claim_pair() {
    let mut fixture = side_effect_fixture(
        HistorySaga::Fail,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::Ambiguous,
    );
    let index = fixture
        .records
        .iter()
        .position(|record| matches!(record.payload(), KernelEventPayload::SideEffectAmbiguous(_)))
        .expect("ambiguity record");
    let mut ambiguity = match fixture.records[index].payload().clone() {
        KernelEventPayload::SideEffectAmbiguous(payload) => payload,
        _ => unreachable!("located ambiguity"),
    };
    let verify_node_id = ambiguity.node_id.clone();
    let verify_attempt_id = ambiguity.attempt_id.clone();
    let unrelated_attempt_id = attempt_id(0x76);
    ambiguity.node_id = fixture.unrelated.node_id.clone();
    ambiguity.attempt_id = unrelated_attempt_id.clone();
    let ambiguity_object_key = fixture
        .objects
        .iter()
        .find(|(_, (_, evidence))| evidence.artifact_role == ArtifactRole::AmbiguityEvidence)
        .map(|(key, _)| key.clone())
        .expect("ambiguity object");
    let (bytes, mut evidence) = fixture
        .objects
        .remove(&ambiguity_object_key)
        .expect("ambiguity object");
    evidence.producer_node_id = Some(fixture.unrelated.node_id.clone());
    ambiguity.evidence_artifact_evidence_hash = evidence
        .evidence_hash()
        .expect("mutated ambiguity evidence");
    fixture
        .objects
        .insert(object_key(&evidence), (bytes, evidence));
    fixture.records[index] = replace_record_payload(
        &fixture.records[index],
        KernelEventPayload::SideEffectAmbiguous(ambiguity),
    );
    let attempt_failed_index = fixture
        .records
        .iter()
        .position(|record| {
            matches!(
                record.payload(),
                KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    node_id,
                    attempt_id,
                    ..
                }) if node_id == &verify_node_id && attempt_id == &verify_attempt_id
            )
        })
        .expect("paired attempt failure");
    let mut attempt_failed = match fixture.records[attempt_failed_index].payload().clone() {
        KernelEventPayload::StateAttemptFailed(payload) => payload,
        _ => unreachable!("located attempt failure"),
    };
    attempt_failed.node_id = fixture.unrelated.node_id.clone();
    attempt_failed.attempt_id = unrelated_attempt_id;
    fixture.records[attempt_failed_index] = replace_record_payload(
        &fixture.records[attempt_failed_index],
        KernelEventPayload::StateAttemptFailed(attempt_failed),
    );
    let attempt_interrupted_index = fixture
        .records
        .iter()
        .position(|record| {
            matches!(
                record.payload(),
                KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
                    node_id,
                    ..
                }) if node_id == &fixture.unrelated.node_id
            )
        })
        .expect("unrelated attempt interruption");
    let mut attempt_interrupted = match fixture.records[attempt_interrupted_index].payload().clone()
    {
        KernelEventPayload::StateAttemptInterrupted(payload) => payload,
        _ => unreachable!("located attempt interruption"),
    };
    attempt_interrupted.node_id = verify_node_id;
    attempt_interrupted.attempt_id = verify_attempt_id;
    fixture.records[attempt_interrupted_index] = replace_record_payload(
        &fixture.records[attempt_interrupted_index],
        KernelEventPayload::StateAttemptInterrupted(attempt_interrupted),
    );
    assert_side_effect_certified_history_rejects(fixture, "non-side-effect node");
}

#[test]
fn certified_history_rejects_non_verify_terminal_release_authority() {
    let mut fixture = side_effect_fixture(
        HistorySaga::Fail,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::Receipt,
    );
    let index = fixture
        .records
        .iter()
        .position(|record| {
            matches!(
                record.payload(),
                KernelEventPayload::ResourceLaneReleased(_)
            )
        })
        .expect("resource lane release");
    let mut release = match fixture.records[index].payload().clone() {
        KernelEventPayload::ResourceLaneReleased(payload) => payload,
        _ => unreachable!("located release"),
    };
    release.release_authority = events::ResourceLaneReleaseAuthority::ManualResolution;
    fixture.records[index] = replace_record_payload(
        &fixture.records[index],
        KernelEventPayload::ResourceLaneReleased(release),
    );
    assert_side_effect_certified_history_rejects(
        fixture,
        "manual resource-lane release requires adjacent ManualResolutionRecorded",
    );
}

#[test]
fn certified_history_rejects_receipt_below_finalized_terminal_policy() {
    let fixture = side_effect_fixture(
        HistorySaga::Fail,
        SideEffectVerificationSpec::Finalized { depth: 1 },
        SideEffectTerminal::Receipt,
    );
    assert_side_effect_certified_history_rejects(fixture, "produced before terminal evidence");
}

#[test]
fn certified_history_rejects_verify_terminal_reordered_before_release() {
    let mut fixture = side_effect_fixture(
        HistorySaga::Fail,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::Receipt,
    );
    let release_index = fixture
        .records
        .iter()
        .position(|record| {
            matches!(
                record.payload(),
                KernelEventPayload::ResourceLaneReleased(_)
            )
        })
        .expect("verify-terminal resource-lane release");
    let receipt_index = fixture
        .records
        .iter()
        .position(|record| {
            matches!(
                record.payload(),
                KernelEventPayload::SideEffectReceiptObserved(_)
            )
        })
        .expect("verify-terminal receipt");
    let release = fixture.records[release_index].payload().clone();
    let receipt = fixture.records[receipt_index].payload().clone();
    fixture.records[release_index] =
        replace_record_payload(&fixture.records[release_index], receipt);
    fixture.records[receipt_index] =
        replace_record_payload(&fixture.records[receipt_index], release);

    assert_side_effect_certified_history_rejects(
        fixture,
        "verify-terminal resource-lane release must immediately precede terminal side-effect evidence",
    );
}

#[test]
fn certified_history_accepts_verify_release_followed_by_submit_failure() {
    let mut fixture = side_effect_fixture(
        HistorySaga::Fail,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::Receipt,
    );
    let release = fixture
        .records
        .iter()
        .find_map(|record| match record.payload() {
            KernelEventPayload::ResourceLaneReleased(payload) => Some(payload.clone()),
            _ => None,
        })
        .expect("verify-terminal resource-lane release");
    let submit_attempt = fixture
        .records
        .iter()
        .find_map(|record| match record.payload() {
            KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == fixture.submit.node_id =>
            {
                Some(payload.attempt_id.clone())
            }
            _ => None,
        })
        .expect("submit attempt");
    fixture.records.retain(|record| record.seq().as_u64() <= 6);
    append_batch(
        &mut fixture.records,
        &fixture.run_id,
        7,
        "submit-failure-terminal",
        vec![
            KernelEventPayload::ResourceLaneReleased(release.clone()),
            KernelEventPayload::SideEffectFailed(side_effect::Failed {
                spec_hash: release.spec_hash.clone(),
                node_id: fixture.submit.node_id.clone(),
                attempt_id: submit_attempt.clone(),
                ledger_key: release.ledger_key,
                ledger_purpose: release.ledger_purpose,
                pair_id: release.pair_id,
                pair_role: events::SideEffectPairRole::Submit,
                invocation_epoch: release.invocation_epoch,
                failure_phase: side_effect::FailurePhase::BeforeInvocationStarted,
                retryable: false,
                error: side_effect_error(),
            }),
            KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                spec_hash: release.spec_hash,
                node_id: fixture.submit.node_id.clone(),
                attempt_id: submit_attempt,
                retryable: false,
                error: side_effect_error(),
            }),
        ],
    );

    let view = verify_side_effect_fixture(fixture)
        .expect("verify-role release may precede submit-role terminal failure");
    assert_eq!(view.current_run_sequence(), Some(7));
}

#[test]
fn certified_history_accepts_pre_intent_side_effect_attempt_failure() {
    let mut fixture = side_effect_fixture(
        HistorySaga::Fail,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::Receipt,
    );
    let submit_attempt = fixture
        .records
        .iter()
        .find_map(|record| match record.payload() {
            KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == fixture.submit.node_id =>
            {
                Some(payload.attempt_id.clone())
            }
            _ => None,
        })
        .expect("submit attempt");
    let spec_hash = fixture
        .records
        .first()
        .expect("run admission")
        .payload()
        .spec_hash()
        .clone();
    fixture.records.retain(|record| record.seq().as_u64() <= 2);
    append_batch(
        &mut fixture.records,
        &fixture.run_id,
        3,
        "pre-intent-attempt-failure",
        vec![KernelEventPayload::StateAttemptFailed(
            events::StateAttemptFailed {
                spec_hash,
                node_id: fixture.submit.node_id.clone(),
                attempt_id: submit_attempt,
                retryable: false,
                error: side_effect_error(),
            },
        )],
    );

    let view = verify_side_effect_fixture(fixture)
        .expect("pre-intent attempt failure has no established pair authority to settle");
    assert_eq!(view.current_run_sequence(), Some(3));
}

#[test]
fn verified_successor_rejects_established_attempt_failure_without_terminal_evidence() {
    let mut fixture = side_effect_fixture(
        HistorySaga::Fail,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::Receipt,
    );
    let submit_attempt = fixture
        .records
        .iter()
        .find_map(|record| match record.payload() {
            KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == fixture.submit.node_id =>
            {
                Some(payload.attempt_id.clone())
            }
            _ => None,
        })
        .expect("submit attempt");
    let spec_hash = fixture
        .records
        .first()
        .expect("run admission")
        .payload()
        .spec_hash()
        .clone();
    let prefix_records = fixture.records[..=2].to_vec();
    let prefix = load_journal(&fixture.run_id, prefix_records, fixture.objects.clone())
        .expect("intent-established prefix journal")
        .verify(fixture.certified)
        .expect("intent-established prefix");
    fixture.records.truncate(3);
    append_batch(
        &mut fixture.records,
        &fixture.run_id,
        4,
        "established-ledger-attempt-failure",
        vec![KernelEventPayload::StateAttemptFailed(
            events::StateAttemptFailed {
                spec_hash,
                node_id: fixture.submit.node_id.clone(),
                attempt_id: submit_attempt,
                retryable: false,
                error: side_effect_error(),
            },
        )],
    );
    let successor = load_journal(&fixture.run_id, fixture.records, fixture.objects)
        .expect("structurally valid successor journal");
    let error = prefix
        .verify_successor(successor)
        .expect_err("established pair requires same-batch terminal evidence");
    assert!(
        matches!(
            &error,
            StoreError::ProjectionConflict { message, .. }
                if message
                    == "side-effect attempt failure requires terminal side-effect evidence in the same commit"
        ),
        "unexpected error: {error:?}"
    );
}

#[test]
fn current_projection_rejects_side_effect_failure_retryability_mismatch() {
    let mut fixture = side_effect_fixture(
        HistorySaga::Fail,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::Ambiguous,
    );
    let attempt_failed_index = fixture
        .records
        .iter()
        .position(|record| matches!(record.payload(), KernelEventPayload::StateAttemptFailed(_)))
        .expect("side-effect attempt failure");
    let mut attempt_failed = match fixture.records[attempt_failed_index].payload().clone() {
        KernelEventPayload::StateAttemptFailed(payload) => payload,
        _ => unreachable!("located attempt failure"),
    };
    attempt_failed.retryable = true;
    attempt_failed.error.retryable = true;
    fixture.records[attempt_failed_index] = replace_record_payload(
        &fixture.records[attempt_failed_index],
        KernelEventPayload::StateAttemptFailed(attempt_failed),
    );

    assert_side_effect_current_projection_rejects(
        fixture,
        "side-effect ambiguity retryability must match attempt failure",
    );
}

#[test]
fn certified_history_accepts_adjacent_manual_release_and_resolution() {
    let fixture = side_effect_fixture(
        HistorySaga::Manual,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::ManualResolution,
    );
    let view = verify_side_effect_fixture(fixture).expect("adjacent manual release and resolution");
    assert_eq!(view.current_run_sequence(), Some(14));
}

#[derive(Debug, Clone, Copy)]
enum InvalidManualAuthorization {
    BadSignature,
    WrongSigner,
    DuplicateSigner,
    InsufficientQuorum,
    WrongVerifier,
    WrongSigningScheme,
    NonCanonicalProof,
    UnknownProofField,
    UnsupportedClaimVersion,
    UnsupportedProofVersion,
    ClaimRun,
    ClaimSpec,
    ClaimExpectedSequence,
    ClaimStreamPrefix,
    ClaimBlockReason,
    ClaimObligations,
    ClaimOutcome,
    ClaimEvidenceSchema,
    ClaimEvidenceContent,
    ClaimEvidenceArtifact,
}

#[test]
fn certified_history_rejects_invalid_manual_authorization_matrix() {
    use InvalidManualAuthorization as Case;

    let cases = [
        Case::BadSignature,
        Case::WrongSigner,
        Case::DuplicateSigner,
        Case::InsufficientQuorum,
        Case::WrongVerifier,
        Case::WrongSigningScheme,
        Case::NonCanonicalProof,
        Case::UnknownProofField,
        Case::UnsupportedClaimVersion,
        Case::UnsupportedProofVersion,
        Case::ClaimRun,
        Case::ClaimSpec,
        Case::ClaimExpectedSequence,
        Case::ClaimStreamPrefix,
        Case::ClaimBlockReason,
        Case::ClaimObligations,
        Case::ClaimOutcome,
        Case::ClaimEvidenceSchema,
        Case::ClaimEvidenceContent,
        Case::ClaimEvidenceArtifact,
    ];
    for case in cases {
        let mut fixture = side_effect_fixture(
            HistorySaga::Manual,
            SideEffectVerificationSpec::Receipt,
            SideEffectTerminal::ManualResolution,
        );
        let mut proof = manual_authorization_proof(&fixture);
        match case {
            Case::BadSignature => {
                proof.signatures[0].signature =
                    mfm_manual_auth::ManualAuthorizationSignatureBytes::new(vec![0x44; 65])
                        .expect("invalid test signature shape");
            }
            Case::WrongSigner => {
                proof.signatures[0].operator_id =
                    spec::OperatorId::new("mfm.store.test.manual.other-operator")
                        .expect("other operator");
            }
            Case::DuplicateSigner => {
                proof.signatures.push(proof.signatures[0].clone());
            }
            Case::InsufficientQuorum => {
                proof.signatures.clear();
            }
            Case::WrongVerifier => {
                proof.verifier_id = spec::ManualAuthorizationVerifierId::new(
                    "mfm.store.test.manual.other-verifier",
                )
                .expect("other verifier");
            }
            Case::WrongSigningScheme => {
                proof.signing_scheme =
                    spec::ManualSigningSchemeSpec::new("mfm.store.test.manual.other-scheme")
                        .expect("other signing scheme");
            }
            Case::NonCanonicalProof => {
                let mut bytes = vec![b' '];
                bytes.extend_from_slice(
                    proof
                        .canonical_json()
                        .expect("canonical manual proof")
                        .as_bytes(),
                );
                replace_manual_authorization_bytes(&mut fixture, bytes);
            }
            Case::UnknownProofField => {
                let canonical = proof.canonical_json().expect("canonical manual proof");
                let mut value: serde_json::Value =
                    serde_json::from_slice(canonical.as_bytes()).expect("manual proof JSON");
                value["unknown"] = serde_json::json!(true);
                let json = serde_json::to_string(&value).expect("manual proof JSON");
                let bytes = PlainCanonicalJsonBytes::from_json_str(&json)
                    .expect("canonical unknown-field proof")
                    .to_vec();
                replace_manual_authorization_bytes(&mut fixture, bytes);
            }
            Case::UnsupportedClaimVersion | Case::UnsupportedProofVersion => {
                let canonical = proof.canonical_json().expect("canonical manual proof");
                let mut value: serde_json::Value =
                    serde_json::from_slice(canonical.as_bytes()).expect("manual proof JSON");
                match case {
                    Case::UnsupportedClaimVersion => {
                        value["claim"]["claim_version"] =
                            serde_json::json!("mfm.manual_resolution.authorization_claim.v2");
                    }
                    Case::UnsupportedProofVersion => {
                        value["proof_version"] =
                            serde_json::json!("mfm.manual_resolution.authorization_proof.v2");
                    }
                    _ => unreachable!("version mutation cases only"),
                }
                let json = serde_json::to_string(&value).expect("manual proof JSON");
                let bytes = PlainCanonicalJsonBytes::from_json_str(&json)
                    .expect("canonical unsupported-version proof")
                    .to_vec();
                replace_manual_authorization_bytes(&mut fixture, bytes);
            }
            Case::ClaimRun => {
                proof.claim.run_id = RunId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([0xd1; 32]),
                );
            }
            Case::ClaimSpec => {
                proof.claim.spec_hash = mfm_ids::SpecHash::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([0xd2; 32]),
                );
            }
            Case::ClaimExpectedSequence => {
                proof.claim.expected_next_seq += 1;
            }
            Case::ClaimStreamPrefix => {
                proof.claim.stream_prefix_digest = fixed_content_digest_for_test(0xd3);
            }
            Case::ClaimBlockReason => {
                proof.claim.manual_block_reason =
                    mfm_manual_auth::ManualResolutionBlockReason::ForwardAmbiguous;
            }
            Case::ClaimObligations => {
                proof.claim.unresolved_obligations_digest = fixed_content_digest_for_test(0xd4);
            }
            Case::ClaimOutcome => {
                proof.claim.outcome = events::ManualResolutionOutcome::ConfirmRemediated;
            }
            Case::ClaimEvidenceSchema => {
                proof.claim.evidence.schema_id =
                    schema_id("mfm.store.test.other_manual_evidence", 0xd5);
            }
            Case::ClaimEvidenceContent => {
                proof.claim.evidence.content_hash = fixed_content_digest_for_test(0xd6);
            }
            Case::ClaimEvidenceArtifact => {
                proof.claim.evidence.artifact_id = ArtifactId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([0xd7; 32]),
                );
            }
        }
        if !matches!(
            case,
            Case::NonCanonicalProof
                | Case::UnknownProofField
                | Case::UnsupportedClaimVersion
                | Case::UnsupportedProofVersion
        ) {
            replace_manual_authorization_proof(&mut fixture, &proof);
        }

        let error =
            verify_side_effect_fixture(fixture).expect_err("invalid manual proof must reject");
        assert!(
            matches!(
                &error,
                StoreError::PersistedEventMismatch {
                    field: "certified_history",
                    message,
                } if message.contains("manual-resolution authorization verification failed")
            ),
            "{case:?} produced unexpected error: {error:?}"
        );
    }
}

#[test]
fn certified_history_rejects_proof_for_a_different_legal_prefix() {
    let mut alternate = side_effect_fixture(
        HistorySaga::Manual,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::Ambiguous,
    );
    let spec_hash = alternate.certified.spec_hash().clone();
    let referenced = alternate
        .objects
        .values()
        .map(|(_, evidence)| evidence)
        .find(|evidence| evidence.artifact_role == ArtifactRole::TypedConfig)
        .expect("typed config evidence")
        .clone();
    let last = alternate.records.last().expect("alternate prefix record");
    alternate.records.push(persisted_record(
        &alternate.run_id,
        last.seq().as_u64(),
        last.store_commit_order().as_u64(),
        1,
        last.commit_key().as_str(),
        KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
            spec_hash,
            node_id: referenced.producer_node_id.clone(),
            attempt_id: None,
            artifact_ref: event_artifact_evidence(&referenced),
        }),
    ));
    let alternate_view =
        verify_side_effect_fixture(alternate).expect("different legal manual prefix");
    let alternate_prefix = mfm_store::v1::current_lifecycle::read(&alternate_view)
        .manual_resolution_prefix_authority()
        .expect("different legal prefix authority");

    let mut fixture = side_effect_fixture(
        HistorySaga::Manual,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::ManualResolution,
    );
    assert_eq!(alternate_prefix.run_id(), &fixture.run_id);
    assert_eq!(alternate_prefix.spec_hash(), fixture.certified.spec_hash());
    assert_eq!(alternate_prefix.expected_next_seq(), 14);
    let proof = signed_manual_proof(
        &alternate_prefix,
        events::ManualResolutionOutcome::FailWithoutAcdcClaim,
        manual_evidence_ref_from_fixture(&fixture),
    );
    replace_manual_authorization_proof(&mut fixture, &proof);

    let error =
        verify_side_effect_fixture(fixture).expect_err("different legal prefix must reject");
    assert!(
        matches!(
            &error,
            StoreError::PersistedEventMismatch {
                field: "certified_history",
                message,
            } if message.contains("manual-resolution authorization verification failed")
        ),
        "unexpected different-prefix error: {error:?}"
    );
}

#[test]
fn verified_successor_accepts_valid_manual_suffix_and_rejects_invalid_manual_suffix() {
    for invalid in [false, true] {
        let mut fixture = side_effect_fixture(
            HistorySaga::Manual,
            SideEffectVerificationSpec::Receipt,
            SideEffectTerminal::ManualResolution,
        );
        if invalid {
            let mut proof = manual_authorization_proof(&fixture);
            proof.signatures[0].signature =
                mfm_manual_auth::ManualAuthorizationSignatureBytes::new(vec![0x55; 65])
                    .expect("invalid suffix signature shape");
            replace_manual_authorization_proof(&mut fixture, &proof);
        }
        let SideEffectHistoryFixture {
            certified,
            run_id,
            records,
            objects,
            ..
        } = fixture;
        let suffix_start = records
            .iter()
            .position(|record| record.seq().as_u64() == 14)
            .expect("manual suffix start");
        let prefix = load_journal(&run_id, records[..suffix_start].to_vec(), objects.clone())
            .expect("manual prefix journal")
            .verify(certified)
            .expect("verified manual prefix");
        let successor = load_journal(&run_id, records, objects).expect("manual successor journal");
        let result = prefix.verify_successor(successor);
        if invalid {
            let error = result.expect_err("invalid manual suffix must reject");
            assert!(
                matches!(
                    &error,
                    StoreError::PersistedEventMismatch {
                        field: "certified_history",
                        message,
                    } if message.contains("manual-resolution authorization verification failed")
                ),
                "unexpected invalid-suffix error: {error:?}"
            );
        } else {
            let view = result.expect("valid manual suffix");
            assert_eq!(view.current_run_sequence(), Some(14));
        }
    }
}

#[test]
fn certified_history_rejects_nonadjacent_manual_resource_release() {
    let mut fixture = side_effect_fixture(
        HistorySaga::Manual,
        SideEffectVerificationSpec::Receipt,
        SideEffectTerminal::ManualResolution,
    );
    let release_index = fixture
        .records
        .iter()
        .position(|record| {
            matches!(
                record.payload(),
                KernelEventPayload::ResourceLaneReleased(events::ResourceLaneReleased {
                    release_authority: events::ResourceLaneReleaseAuthority::ManualResolution,
                    ..
                })
            )
        })
        .expect("manual release");
    let manual_index = fixture
        .records
        .iter()
        .position(|record| {
            matches!(
                record.payload(),
                KernelEventPayload::ManualResolutionRecorded(_)
            )
        })
        .expect("manual resolution");
    let manual = match fixture.records[manual_index].payload() {
        KernelEventPayload::ManualResolutionRecorded(manual) => manual,
        _ => unreachable!("located manual resolution"),
    };
    let evidence_key = (
        manual.evidence_artifact_id.clone(),
        manual.evidence_artifact_evidence_hash.clone(),
    );
    let evidence = &fixture
        .objects
        .get(&evidence_key)
        .expect("manual evidence object")
        .1;
    let separating_reference = KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
        spec_hash: manual.spec_hash.clone(),
        node_id: None,
        attempt_id: None,
        artifact_ref: events::ArtifactEvidenceRef {
            artifact_id: evidence.artifact_id.clone(),
            role: evidence.artifact_role,
            schema_id: evidence.schema_id.clone().expect("manual evidence schema"),
            semantic_type_id: evidence.semantic_type_id.clone(),
            content_digest: evidence.digest.clone(),
            evidence_hash: evidence.evidence_hash().expect("manual evidence identity"),
            byte_len: evidence.byte_len,
            media_type: evidence.media_type.clone(),
        },
    });
    let manual_payload = fixture.records[manual_index].payload().clone();
    fixture.records[manual_index] = persisted_record(
        &fixture.run_id,
        14,
        14,
        1,
        "manual-resolution",
        separating_reference,
    );
    fixture.records.insert(
        manual_index + 1,
        persisted_record(
            &fixture.run_id,
            14,
            14,
            2,
            "manual-resolution",
            manual_payload,
        ),
    );
    assert_eq!(fixture.records[release_index].ordinal().as_u32(), 0);
    assert_side_effect_certified_history_rejects(
        fixture,
        "manual resource-lane release requires adjacent ManualResolutionRecorded",
    );
}
