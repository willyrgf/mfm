use super::*;

use mfm_capabilities::{CapabilitySpec, ExternalMutationAuthorityRole};
use mfm_events::v1::side_effect;
use mfm_ids::{DigestBytes, SemanticTypeId};
use mfm_program::{
    build_root_with_registries, AdapterBindingSpec, CanonicalSeed, NoContext, PublicOutputKey,
    ResourceClaim, RootBuilder, ScopeKey, SideEffectState, StateKey, StateRegistryBuilder,
    StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.storage.postgres.test",
    name = "manual_journal_value",
    version = "1",
    schema = "mfm.storage.postgres.test.manual_journal_value"
)]
struct ManualJournalValue {
    amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct ManualJournalConfig {
    multiplier: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.storage.postgres.test.manual_journal_outputs")]
struct ManualJournalOutputs<'program, 'scope> {
    result: mfm_program::Handle<'program, 'scope, ManualJournalValue>,
}

struct ManualJournalCapability;

impl CapabilitySpec for ManualJournalCapability {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.storage.postgres.test",
            "manual_journal_mutation",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x91; 32]),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.storage.postgres.test.manual_journal_mutation.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.storage.postgres.test.manual_journal_mutation"
    }
}

struct ManualJournalState {
    config: ManualJournalConfig,
}

impl StateSpec for ManualJournalState {
    type Config = ManualJournalConfig;
    type Context = NoContext;
    type Input = ManualJournalValue;
    type Output = ManualJournalValue;
    type Effect = mfm_capabilities::ApplySideEffect;
    type Caps = (ManualJournalCapability,);

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        mfm_ids::StateKind::new(
            "mfm.storage.postgres.test",
            "manual_journal_state",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x92; 32]),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.storage.postgres.test.manual_journal_state.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.storage.postgres.test.manual_journal_state"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        Ok(vec![AdapterBindingSpec {
            adapter_kind: manual_adapter_kind(),
            adapter_version: manual_adapter_version(),
        }])
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl SideEffectState for ManualJournalState {
    type Intent = ManualJournalValue;
    type IdempotencyInput = ManualJournalValue;
    type PreparedInvocation = ManualJournalValue;
    type Submission = ManualJournalValue;
    type RecoveryEvidence = ManualJournalValue;
    type Receipt = ManualJournalValue;
    type Confirmation = ManualJournalValue;

    fn intent(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<mfm_program::SideEffectIntent<Self::Intent, Self::IdempotencyInput>> {
        let intent = ManualJournalValue {
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

fn manual_adapter_kind() -> AdapterKind {
    AdapterKind::new(
        "mfm.storage.postgres.test",
        "manual_journal_adapter",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x93; 32]),
    )
    .expect("manual journal adapter kind")
}

fn manual_adapter_version() -> AdapterVersion {
    AdapterVersion::new("mfm.storage.postgres.test.manual_journal_adapter.v1")
        .expect("manual journal adapter version")
}

fn manual_evidence_schema() -> SchemaId {
    SchemaId::new(
        "mfm.storage.postgres.test.manual_evidence",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x94; 32]),
    )
    .expect("manual evidence schema")
}

fn manual_resolution_policy() -> (
    mfm_program::SideEffectSagaPolicy,
    spec::ManualResolutionEvidenceSpec,
) {
    let operator = mfm_program::OperatorAuthorityMemberSpec {
        operator_id: mfm_program::OperatorId::new("mfm.storage.postgres.test.manual.operator")
            .expect("operator id"),
        public_identity: mfm_program::OperatorPublicIdentity::new(
            "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
        )
        .expect("operator public identity"),
    };
    let authority = mfm_program::OperatorAuthoritySnapshotDraft::new(
        mfm_program::OperatorAuthorityId::new("mfm.storage.postgres.test.manual.authority")
            .expect("authority id"),
        mfm_program::NonEmptyUniqueOperators::new(operator, Vec::new())
            .expect("operator authority"),
    );
    let authorization = mfm_program::ManualAuthorizationDraft::threshold(
        mfm_program::ManualAuthorizationVerifierId::new(
            "mfm.storage.postgres.test.manual.verifier",
        )
        .expect("manual verifier"),
        mfm_program::ManualSigningSchemeSpec::new("mfm.manual_resolution.digest_signature.v1")
            .expect("manual signing scheme"),
        authority,
        mfm_program::ThresholdQuorum::new(1).expect("manual quorum"),
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

fn certified_manual_journal_spec() -> mfm_certify::CertifiedTypedSpec {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<ManualJournalState>()
        .expect("manual journal state registration");
    let (manual_policy, manual_spec) = manual_resolution_policy();
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root scope"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(manual_policy)?;
            let input = root.seed(
                mfm_program::SeedKey::new("initial")?,
                CanonicalSeed::from_value(&ManualJournalValue { amount: 7 })?,
            )?;
            let result = root.scope().side_effect::<ManualJournalState, _>(
                StateKey::new("manual-journal-side-effect")?,
                NoContext,
                ManualJournalConfig { multiplier: 2 },
                input,
                ResourceClaim::manual_only(),
                mfm_program::SideEffectVerificationSpec::Receipt,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &ManualJournalOutputs {
                    result: result.into_handle(),
                },
            )
        },
    )
    .expect("manual journal draft");
    let mut registry =
        mfm_certify::CertificationRegistry::from_program_draft(&draft).expect("registry");
    registry
        .register_schema_role(
            manual_spec.evidence_schema.clone(),
            mfm_certify::CertifiedSchemaRole::ManualResolutionEvidence,
        )
        .expect("manual evidence role");
    registry
        .register_manual_authorization_verifier(manual_spec.authorization.verifier_id.clone())
        .expect("manual verifier");
    registry
        .register_operator_authority_snapshot(manual_spec.authorization.authority.clone())
        .expect("manual authority");
    let lowered = mfm_certify::lower_program_draft(&draft).expect("lowered manual journal");
    mfm_certify::certify_typed_spec(lowered, &registry).expect("certified manual journal")
}

struct ManualJournalFixture {
    run_id: RunId,
    records: Vec<KernelEventEnvelope>,
    objects: ArtifactByteAuthorityMap,
}

fn exact_artifact(
    bytes: &[u8],
    role: ArtifactRole,
    schema_id: SchemaId,
    media_type: MediaType,
    semantic_type_id: Option<SemanticTypeId>,
    producer_node_id: Option<NodeId>,
) -> ArtifactEvidenceRef {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type,
        schema_id: Some(schema_id),
        semantic_type_id,
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn object_key(evidence: &ArtifactEvidenceRef) -> (ArtifactId, ContentDigest) {
    (
        evidence.artifact_id.clone(),
        evidence.evidence_hash().expect("artifact evidence hash"),
    )
}

fn manual_object(
    bytes: &[u8],
    role: ArtifactRole,
    schema_id: SchemaId,
    producer_node_id: Option<NodeId>,
) -> (Vec<u8>, ArtifactEvidenceRef) {
    let json = std::str::from_utf8(bytes).expect("UTF-8 manual journal object");
    let bytes = PlainCanonicalJsonBytes::from_json_str(json)
        .expect("canonical manual journal object")
        .to_vec();
    let evidence = exact_artifact(
        &bytes,
        role,
        schema_id,
        MediaType::new("application/json").expect("JSON media type"),
        None,
        producer_node_id,
    );
    (bytes, evidence)
}

fn admitted_manual_journal_run(
    certified: &mfm_certify::CertifiedTypedSpec,
    store_scope_byte: u8,
) -> (events::RunAdmitted, ArtifactByteAuthorityMap, RunId) {
    let spec = &certified.envelope().spec;
    let config_json =
        serde_json::to_string(&ManualJournalConfig { multiplier: 2 }).expect("config JSON");
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
                    sha256_digest_bytes(&bytes),
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

    let seed_spec = spec
        .seeds
        .first()
        .cloned()
        .expect("one manual journal seed");
    let seed =
        CanonicalSeed::from_value(&ManualJournalValue { amount: 7 }).expect("canonical seed");
    let seed_bytes = seed.canonical_json().as_bytes().to_vec();
    let seed_evidence = ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(
            seed.content_digest().algorithm(),
            *seed.content_digest().digest(),
        ),
        digest: seed.content_digest().clone(),
        byte_len: seed.byte_len() as u64,
        media_type: MediaType::new("application/json").expect("JSON media type"),
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
    let identity_material = run_identity_material_for_test(
        certified.spec_hash().clone(),
        &format!("{store_scope_byte:02x}").repeat(16),
    );
    let run_id = identity_material
        .derive_run_id()
        .expect("manual journal run id");
    let admitted = events::RunAdmitted {
        run_id: run_id.clone(),
        identity_material,
        entry_point: events::EntryPointLaunchEvidence::new(
            "mfm.storage.postgres.test/manual_journal@1",
            Vec::new(),
        )
        .expect("entry point"),
        spec_hash: certified.spec_hash().clone(),
        spec_artifact: run_artifact_ref(&spec_evidence),
        certificate_artifact: run_artifact_ref(&certificate_evidence),
        config_artifacts: config_objects
            .iter()
            .map(|(_, evidence)| run_artifact_ref(evidence))
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
        admitted_binding_digest: content_digest(0x9f),
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

fn fixture_schema(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
    .expect("fixture schema")
}

fn persisted_record(
    run_id: &RunId,
    sequence: u64,
    store_commit_order: u64,
    ordinal: u32,
    commit_key: &str,
    payload: KernelEventPayload,
) -> KernelEventEnvelope {
    mfm_store::v1::test_support::persisted_kernel_event_envelope_with_ordinal_for_test(
        run_id,
        sequence,
        store_commit_order,
        ordinal,
        CommitKey::new(commit_key).expect("commit key"),
        payload,
    )
}

fn append_batch(
    records: &mut Vec<KernelEventEnvelope>,
    run_id: &RunId,
    sequence: u64,
    store_commit_order: u64,
    commit_key: &str,
    payloads: Vec<KernelEventPayload>,
) {
    for (ordinal, payload) in payloads.into_iter().enumerate() {
        records.push(persisted_record(
            run_id,
            sequence,
            store_commit_order,
            ordinal as u32,
            commit_key,
            payload,
        ));
    }
}

fn attempt_started(
    run_id: &RunId,
    sequence: u64,
    store_commit_order: u64,
    spec_hash: &SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    commit_key: &str,
) -> KernelEventEnvelope {
    persisted_record(
        run_id,
        sequence,
        store_commit_order,
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

fn manual_journal_error() -> events::MfmErrorInfo {
    events::MfmErrorInfo {
        code: events::ErrorCode::new("manual_journal_ambiguous").expect("error code"),
        category: events::ErrorCategory::SideEffect,
        retryable: false,
        safe_message: "side effect outcome is ambiguous".to_owned(),
        public_details: None,
        diagnostic_ref: None,
    }
}

fn manual_signing_key() -> k256::ecdsa::SigningKey {
    let mut key_bytes = [0_u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("manual fixture key");
    k256::ecdsa::SigningKey::from(&secret_key)
}

fn sign_manual_claim_digest(signing_key: &k256::ecdsa::SigningKey, digest: &[u8; 32]) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual fixture signature");
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
        &manual_signing_key(),
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

fn build_manual_journal_fixture(
    store_scope_byte: u8,
    first_store_commit_order: u64,
    invalid_signature: bool,
) -> ManualJournalFixture {
    let certified = certified_manual_journal_spec();
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
    let submit_cell = spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == submit.output_cell)
        .cloned()
        .expect("submit output cell");
    let submit_attempt = attempt_id(0xa1);
    let verify_attempt = attempt_id(0xa2);
    let ledger_key = events::SideEffectLedgerKey::new("manual-journal-ledger").expect("ledger key");
    let ledger_purpose = events::SideEffectLedgerPurpose::Forward;
    let runner =
        events::RunnerInvocationId::new("manual-journal-runner").expect("runner invocation");
    let fencing_token =
        side_effect::ClaimFencingToken::new("manual-journal-fence").expect("fencing token");
    let (admitted, mut objects, run_id) = admitted_manual_journal_run(&certified, store_scope_byte);
    let order = |sequence: u64| first_store_commit_order + sequence - 1;
    let mut records = vec![persisted_record(
        &run_id,
        1,
        order(1),
        0,
        "manual-journal-admission",
        KernelEventPayload::RunAdmitted(Box::new(admitted)),
    )];
    records.push(attempt_started(
        &run_id,
        2,
        order(2),
        &spec_hash,
        &submit,
        &submit_attempt,
        "manual-journal-submit-start",
    ));

    let intent = manual_object(
        br#"{"amount":14}"#,
        ArtifactRole::SideEffectIntent,
        fixture_schema("mfm.storage.postgres.test.manual_intent", 0xa3),
        Some(submit.node_id.clone()),
    );
    let prepared = manual_object(
        br#"{"amount":14}"#,
        ArtifactRole::PreparedInvocation,
        fixture_schema("mfm.storage.postgres.test.manual_prepared", 0xa4),
        Some(submit.node_id.clone()),
    );
    let submission = manual_object(
        br#"{"amount":14}"#,
        ArtifactRole::Submission,
        fixture_schema("mfm.storage.postgres.test.manual_submission", 0xa5),
        Some(submit.node_id.clone()),
    );
    for (bytes, evidence) in [&intent, &prepared, &submission] {
        objects.insert(object_key(evidence), (bytes.clone(), evidence.clone()));
    }

    append_batch(
        &mut records,
        &run_id,
        3,
        order(3),
        "manual-journal-intent",
        vec![KernelEventPayload::SideEffectIntentPersisted(
            side_effect::IntentPersisted {
                spec_hash: spec_hash.clone(),
                node_id: submit.node_id.clone(),
                scope_id: submit.scope_id.clone(),
                attempt_id: submit_attempt.clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role: events::SideEffectPairRole::Submit,
                invocation_epoch: 1,
                intent_schema_id: intent.1.schema_id.clone().expect("intent schema"),
                intent_hash: intent.1.digest.clone(),
                intent_artifact_id: intent.1.artifact_id.clone(),
                intent_artifact_evidence_hash: intent.1.evidence_hash().expect("intent evidence"),
                idempotency_input_schema_id: fixture_schema(
                    "mfm.storage.postgres.test.manual_idempotency_input",
                    0xa6,
                ),
                idempotency_input_hash: content_digest(0xa7),
                idempotency_key: events::IdempotencyKeyRef::new("manual-journal-idempotency")
                    .expect("idempotency key"),
                capability_kind: ManualJournalCapability::kind().expect("capability kind"),
                capability_version: ManualJournalCapability::version().expect("capability version"),
                adapter_kind: manual_adapter_kind(),
                adapter_version: manual_adapter_version(),
            },
        )],
    );
    append_batch(
        &mut records,
        &run_id,
        4,
        order(4),
        "manual-journal-claim",
        vec![KernelEventPayload::SideEffectClaimed(
            side_effect::Claimed {
                spec_hash: spec_hash.clone(),
                node_id: submit.node_id.clone(),
                attempt_id: submit_attempt.clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role: events::SideEffectPairRole::Submit,
                claim_owner: runner.clone(),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: fencing_token.clone(),
            },
        )],
    );
    append_batch(
        &mut records,
        &run_id,
        5,
        order(5),
        "manual-journal-prepared",
        vec![KernelEventPayload::SideEffectInvocationPrepared(
            side_effect::InvocationPrepared {
                spec_hash: spec_hash.clone(),
                node_id: submit.node_id.clone(),
                attempt_id: submit_attempt.clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role: events::SideEffectPairRole::Submit,
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: fencing_token.clone(),
                resource_key: None,
                prepared_schema_id: prepared.1.schema_id.clone().expect("prepared schema"),
                prepared_artifact_id: prepared.1.artifact_id.clone(),
                prepared_hash: prepared.1.digest.clone(),
                prepared_artifact_evidence_hash: prepared
                    .1
                    .evidence_hash()
                    .expect("prepared evidence"),
            },
        )],
    );
    append_batch(
        &mut records,
        &run_id,
        6,
        order(6),
        "manual-journal-invocation",
        vec![KernelEventPayload::SideEffectInvocationStarted(
            side_effect::InvocationStarted {
                spec_hash: spec_hash.clone(),
                node_id: submit.node_id.clone(),
                attempt_id: submit_attempt.clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role: events::SideEffectPairRole::Submit,
                invocation_epoch: 1,
                claim_owner: runner,
                claim_generation: 1,
                claim_fencing_token: fencing_token,
            },
        )],
    );
    append_batch(
        &mut records,
        &run_id,
        7,
        order(7),
        "manual-journal-submission",
        vec![KernelEventPayload::SideEffectSubmissionObserved(
            side_effect::SubmissionObserved {
                spec_hash: spec_hash.clone(),
                node_id: submit.node_id.clone(),
                attempt_id: submit_attempt.clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
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
            },
        )],
    );
    append_batch(
        &mut records,
        &run_id,
        8,
        order(8),
        "manual-journal-submit-terminal",
        vec![
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
                    code: events::ErrorCode::new("submission_boundary").expect("skip reason code"),
                    safe_message: "submission boundary reached".to_owned(),
                },
            }),
            KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: spec_hash.clone(),
                node_id: submit.node_id.clone(),
                attempt_id: submit_attempt,
                output_cell_id: submit_cell.cell_id,
            }),
        ],
    );
    records.push(attempt_started(
        &run_id,
        9,
        order(9),
        &spec_hash,
        &verify,
        &verify_attempt,
        "manual-journal-verify-start",
    ));

    let ambiguity = manual_object(
        br#"{"ambiguous":true}"#,
        ArtifactRole::AmbiguityEvidence,
        fixture_schema("mfm.storage.postgres.test.manual_ambiguity", 0xa8),
        Some(verify.node_id.clone()),
    );
    objects.insert(
        object_key(&ambiguity.1),
        (ambiguity.0.clone(), ambiguity.1.clone()),
    );
    append_batch(
        &mut records,
        &run_id,
        10,
        order(10),
        "manual-journal-ambiguous",
        vec![
            KernelEventPayload::SideEffectAmbiguous(side_effect::Ambiguous {
                spec_hash: spec_hash.clone(),
                node_id: verify.node_id.clone(),
                attempt_id: verify_attempt.clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose,
                pair_id,
                pair_role: events::SideEffectPairRole::Verify,
                invocation_epoch: 1,
                ambiguity_code: events::AmbiguityCode::new("manual_journal_ambiguous")
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
                node_id: verify.node_id,
                attempt_id: verify_attempt,
                retryable: false,
                error: manual_journal_error(),
            }),
        ],
    );

    let prefix_backend = mfm_store::v1::test_support::StaticRunJournalBackendForTest::new(
        run_id.clone(),
        records.clone(),
        objects.clone(),
    );
    let prefix_journal = mfm_store::v1::test_support::poll_ready_store_future_for_test(
        prefix_backend.load_committed_journal(&run_id),
    )
    .expect("physical manual prefix");
    let prefix_view = prefix_journal
        .verify(certified)
        .expect("verified manual prefix");
    let prefix = mfm_store::v1::current_lifecycle::read(&prefix_view)
        .manual_resolution_prefix_authority()
        .expect("manual prefix authority");
    let evidence = manual_object(
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
    let mut proof = signed_manual_proof(&prefix, outcome, evidence_ref);
    if invalid_signature {
        proof.signatures[0].signature =
            mfm_manual_auth::ManualAuthorizationSignatureBytes::new(vec![0x55; 65])
                .expect("invalid signature shape");
    }
    let authorization_bytes = proof
        .canonical_json()
        .expect("canonical manual authorization")
        .to_vec();
    let authorization = manual_object(
        &authorization_bytes,
        ArtifactRole::ManualResolutionAuthorization,
        mfm_manual_auth::manual_authorization_proof_schema_id()
            .expect("manual authorization schema"),
        None,
    );
    for (bytes, artifact) in [&evidence, &authorization] {
        objects.insert(object_key(artifact), (bytes.clone(), artifact.clone()));
    }
    append_batch(
        &mut records,
        &run_id,
        11,
        order(11),
        "manual-journal-resolution",
        vec![KernelEventPayload::ManualResolutionRecorded(
            events::ManualResolutionRecorded {
                run_id: run_id.clone(),
                spec_hash,
                outcome,
                evidence_schema_id: evidence
                    .1
                    .schema_id
                    .clone()
                    .expect("manual evidence schema"),
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
                    events::ManualResolutionNote::new("manual journal resolution")
                        .expect("manual note"),
                ),
            },
        )],
    );
    ManualJournalFixture {
        run_id,
        records,
        objects,
    }
}

fn seed_memory_journal(
    store: &mfm_store::v1::AsyncInMemoryRunStore,
    fixture: &ManualJournalFixture,
) {
    let requirements = fixture
        .records
        .iter()
        .flat_map(|record| event_artifact_requirements(record.payload()))
        .map(|requirement| {
            (
                (
                    requirement.artifact_id.clone(),
                    requirement.evidence_hash.clone(),
                ),
                requirement,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let retained = fixture
        .objects
        .iter()
        .map(|(key, (bytes, evidence))| {
            let requirement = requirements
                .get(key)
                .expect("fixture object is referenced by the journal");
            VerifiedRetainedArtifactBytes::new(bytes.clone(), evidence.clone(), requirement)
                .expect("verified fixture object")
        })
        .collect::<Vec<_>>();
    // This empty projection only admits the exact retained bytes. The records
    // seeded below remain the sole source of journal semantics.
    store
        .seed_projection_snapshot_for_test(ProjectionSnapshot::default(), retained)
        .expect("seed exact in-memory objects");

    let mut commits = BTreeMap::<u64, Vec<KernelEventEnvelope>>::new();
    for record in &fixture.records {
        commits
            .entry(record.seq().as_u64())
            .or_default()
            .push(record.clone());
    }
    for records in commits.into_values() {
        store
            .seed_run_stream_envelopes_for_test(records)
            .expect("seed exact in-memory commit");
    }
}

async fn seed_postgres_journal(
    store: &PostgresStore,
    fixture: &ManualJournalFixture,
) -> Result<()> {
    let mut tx =
        store.pool.begin().await.map_err(|error| {
            database_error("failed to begin journal fixture transaction", error)
        })?;
    for (bytes, evidence) in fixture.objects.values() {
        let artifact = PreparedArtifactBytes::new(bytes.clone(), evidence.clone())?;
        insert_prepared_artifact_bytes_tx(&mut tx, &artifact).await?;
    }

    let all_evidence = fixture
        .objects
        .values()
        .map(|(_, evidence)| evidence.clone())
        .collect::<Vec<_>>();
    let mut commits = BTreeMap::<u64, Vec<KernelEventEnvelope>>::new();
    for record in &fixture.records {
        commits
            .entry(record.seq().as_u64())
            .or_default()
            .push(record.clone());
    }
    let mut greatest_store_order = 0_u64;
    for records in commits.into_values() {
        let first = records.first().expect("non-empty fixture commit");
        let seq = first.seq();
        let commit_key = first.commit_key();
        let store_commit_order = first.store_commit_order();
        greatest_store_order = greatest_store_order.max(store_commit_order.as_u64());
        let fingerprint = CommitFingerprint::from_digest(content_digest(
            u8::try_from(seq.as_u64()).expect("fixture sequence fits in one byte"),
        ));
        let purpose = "journal_fixture";
        let commit_id = mfm_store::v1::backend::derive_commit_id(
            &fixture.run_id,
            seq,
            commit_key,
            purpose,
            &fingerprint,
        )?;
        let admitted = if seq.as_u64() == 1 {
            all_evidence.as_slice()
        } else {
            &[]
        };
        let final_authority = final_commit_authority_from_parts(
            &fixture.run_id,
            seq,
            commit_key,
            &commit_id,
            &records,
            &[],
            admitted,
        )?;
        sqlx::query(
            "INSERT INTO commits \
             (commit_id, run_id, seq, commit_key, commit_purpose, \
              prepared_commit_plan_fingerprint, commit_batch_hash, store_commit_order, \
              event_count) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(&commit_id)
        .bind(fixture.run_id.as_str())
        .bind(u64_to_i64(seq.as_u64(), "commits.seq")?)
        .bind(commit_key.as_str())
        .bind(purpose)
        .bind(fingerprint.as_digest().as_str())
        .bind(&final_authority.commit_batch_hash)
        .bind(u64_to_i64(
            store_commit_order.as_u64(),
            "commits.store_commit_order",
        )?)
        .bind(i32::try_from(records.len()).map_err(|_| {
            PostgresStoreError::Corruption("fixture event count overflow".to_owned())
        })?)
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to insert journal fixture commit", error))?;

        if seq.as_u64() == 1 {
            for evidence in &all_evidence {
                link_run_artifact_tx(
                    &mut tx,
                    &fixture.run_id,
                    commit_key,
                    seq,
                    &commit_id,
                    evidence,
                )
                .await?;
            }
        }
        for event in records {
            let payload = payload_canonical_bytes(event.payload())?;
            sqlx::query(
                "INSERT INTO run_events \
                 (run_id, seq, ordinal, commit_id, event_id, event_schema_id, spec_hash, \
                  commit_key, logical_key, payload_hash, payload_canonical_json) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            )
            .bind(event.run_id().as_str())
            .bind(u64_to_i64(event.seq().as_u64(), "run_events.seq")?)
            .bind(i32::try_from(event.ordinal().as_u32()).map_err(|_| {
                PostgresStoreError::Corruption("fixture event ordinal overflow".to_owned())
            })?)
            .bind(&commit_id)
            .bind(event.event_id().as_str())
            .bind(event.event_schema_id().as_str())
            .bind(event.spec_hash().as_str())
            .bind(event.commit_key().as_str())
            .bind(event.logical_key().as_str())
            .bind(event.payload_hash().as_str())
            .bind(payload.as_bytes())
            .execute(&mut *tx)
            .await
            .map_err(|error| database_error("failed to insert journal fixture event", error))?;
        }
    }
    sqlx::query(
        "UPDATE store_commit_order SET current_order = GREATEST(current_order, $1) \
         WHERE singleton",
    )
    .bind(u64_to_i64(
        greatest_store_order,
        "store_commit_order.current_order",
    )?)
    .execute(&mut *tx)
    .await
    .map_err(|error| database_error("failed to advance journal fixture order", error))?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit journal fixture", error))
}

fn physical_journal_observation(
    journal: &CommittedRunJournal,
) -> (
    RunId,
    Option<u64>,
    mfm_ids::ContentRef,
    Vec<u8>,
    mfm_ids::ContentRef,
    Vec<u8>,
) {
    let (spec_ref, spec_bytes) = journal.certified_spec_object();
    let (certificate_ref, certificate_bytes) = journal.certificate_object();
    (
        journal.run_id().clone(),
        journal.current_run_sequence(),
        spec_ref.clone(),
        spec_bytes.to_vec(),
        certificate_ref.clone(),
        certificate_bytes.to_vec(),
    )
}

fn assert_invalid_manual_authorization(error: &StoreError) {
    assert!(
        matches!(
            error,
            StoreError::PersistedEventMismatch {
                field: "certified_history",
                message,
            } if message.contains("manual-resolution authorization verification failed")
        ),
        "unexpected invalid manual authorization error: {error:?}"
    );
}

#[tokio::test]
async fn memory_and_postgres_share_historical_manual_authorization_verification() {
    let valid = build_manual_journal_fixture(0xb1, 1, false);
    let invalid = build_manual_journal_fixture(0xb2, 100, true);
    let memory = mfm_store::v1::AsyncInMemoryRunStore::default();
    seed_memory_journal(&memory, &valid);
    seed_memory_journal(&memory, &invalid);
    let (postgres, schema) = test_store().await;
    seed_postgres_journal(&postgres, &valid)
        .await
        .expect("seed valid PostgreSQL journal");
    seed_postgres_journal(&postgres, &invalid)
        .await
        .expect("seed invalid PostgreSQL journal");

    let memory_valid = memory
        .load_committed_journal(&valid.run_id)
        .await
        .expect("physically load valid in-memory journal");
    let postgres_valid = postgres
        .load_committed_journal(&valid.run_id)
        .await
        .expect("physically load valid PostgreSQL journal");
    assert_eq!(
        physical_journal_observation(&memory_valid),
        physical_journal_observation(&postgres_valid)
    );
    assert_eq!(
        memory_valid
            .verify(certified_manual_journal_spec())
            .expect("verify valid in-memory manual history")
            .current_run_sequence(),
        Some(11)
    );
    assert_eq!(
        postgres_valid
            .verify(certified_manual_journal_spec())
            .expect("verify valid PostgreSQL manual history")
            .current_run_sequence(),
        Some(11)
    );

    let memory_invalid = memory
        .load_committed_journal(&invalid.run_id)
        .await
        .expect("physical in-memory load is signature-blind");
    let postgres_invalid = postgres
        .load_committed_journal(&invalid.run_id)
        .await
        .expect("physical PostgreSQL load is signature-blind");
    assert_eq!(
        physical_journal_observation(&memory_invalid),
        physical_journal_observation(&postgres_invalid)
    );
    let memory_error = memory_invalid
        .verify(certified_manual_journal_spec())
        .expect_err("in-memory semantic verification must reject the bad signature");
    let postgres_error = postgres_invalid
        .verify(certified_manual_journal_spec())
        .expect_err("PostgreSQL semantic verification must reject the bad signature");
    assert_invalid_manual_authorization(&memory_error);
    assert_invalid_manual_authorization(&postgres_error);

    drop_schema(&postgres, &schema).await;
}
