use super::*;

use std::ops::ControlFlow;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilitySpec, ReadExternal, ReadExternalRole};
use mfm_facts::{FactFieldId, FactOrderingName, MfmFactType as _};
use mfm_ids::{
    CapabilityKind, CapabilityVersion, DigestAlgorithm, DigestBytes, RunId, SemanticTypeId,
    SpecHash, StateKind, StateVersion, StoreScopeId,
};
use mfm_program::{
    build_root_with_registries, AdapterBindingSpec, CanonicalSeed, ExternalReadEvidenceSet,
    NoContext, PublicOutputKey, ReadState, RootBuilder, ScopeKey, StateKey, StateRegistryBuilder,
    StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmFactType, MfmValue, PublicOutputs};
use mfm_store::v1::test_support::{
    fixed_content_digest_for_test, persisted_kernel_event_envelope_for_test,
    persisted_kernel_event_envelope_with_ordinal_for_test, poll_ready_store_future_for_test,
    run_artifact_ref_from_store_artifact_for_test, run_identity_material_for_test,
    StaticRunJournalBackendForTest,
};
use mfm_store::v1::RunJournalStore as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.replay.test",
    name = "fact_value",
    version = "1",
    schema = "mfm.replay.test.fact_value"
)]
pub(super) struct ReplayFactValue {
    amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.replay.test",
    name = "fact_subject",
    version = "1",
    schema = "mfm.replay.test.fact_subject"
)]
pub(super) struct ReplayFactSubject {
    account: String,
}

#[allow(clippy::duplicated_attributes)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.replay.test",
    name = "stream_fact",
    version = "1",
    schema = "mfm.replay.test.stream_fact"
)]
#[mfm_fact(kind = "mfm.replay.test.stream")]
#[mfm_fact(field(
    id = "subject.account",
    source = "subject",
    path = "account",
    value_type = "string",
    operator = "equal",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.amount",
    source = "result",
    path = "amount",
    value_type = "unsigned_integer",
    operators(equal, greater_than),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(ordering(
    name = "result.amount.asc",
    term(field = "result.amount", direction = "ascending", nulls = "last")
))]
pub(super) struct ReplayStreamFact {
    subject: ReplayFactSubject,
    response: ReplayFactValue,
}

impl ReplayStreamFact {
    pub(super) fn new(account: &str, amount: u64) -> Self {
        Self {
            subject: ReplayFactSubject {
                account: account.to_owned(),
            },
            response: ReplayFactValue { amount },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct ReplayFactConfig {
    multiplier: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.replay.test.fact_outputs")]
struct ReplayFactOutputs<'program, 'scope> {
    result: mfm_program::Handle<'program, 'scope, ReplayFactValue>,
}

struct ReplayFactReadCapability;

impl CapabilitySpec for ReplayFactReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.replay.test",
            "fact_read",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x21; 32]),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.replay.test.fact_read.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.replay.test.fact_read"
    }
}

struct ReplayFactState {
    config: ReplayFactConfig,
}

impl StateSpec for ReplayFactState {
    type Config = ReplayFactConfig;
    type Context = NoContext;
    type Input = ReplayFactValue;
    type Output = ReplayFactValue;
    type Effect = ReadExternal;
    type Caps = (ReplayFactReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.replay.test",
            "fact_state",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x22; 32]),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.replay.test.fact_state.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.replay.test.fact_state"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        Ok(vec![AdapterBindingSpec {
            adapter_kind: mfm_ids::AdapterKind::new(
                "mfm.replay.test",
                "fact_adapter",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x23; 32]),
            )
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            adapter_version: mfm_ids::AdapterVersion::new("mfm.replay.test.fact_adapter.v1")
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
        }])
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for ReplayFactState {
    type Plan = ReplayFactValue;
    type Evidence = ReplayFactValue;
    type Facts = mfm_values::NonEmpty<ReplayStreamFact>;

    fn plan(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        Ok(ReplayFactValue {
            amount: input.amount * self.config.multiplier,
        })
    }

    fn reduce(
        &self,
        input: &Self::Input,
        evidence: &ExternalReadEvidenceSet<Self::Evidence>,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<(Self::Output, Self::Facts)> {
        let output = self.plan(input, context)?;
        if evidence.primary_evidence() != &output {
            return Err(mfm_program::StateError::Message(
                "replay fact fixture evidence mismatch".to_owned(),
            ));
        }
        Ok((
            output,
            mfm_values::NonEmpty::new(ReplayStreamFact::new("fixture", 1), Vec::new()),
        ))
    }
}

#[derive(Debug, Clone)]
pub(super) enum QuerySource {
    None,
    FirstRecorded,
    External(Box<mfm_facts::InternalFactRef>),
}

impl QuerySource {
    pub(super) fn external(fact_ref: mfm_facts::InternalFactRef) -> Self {
        Self::External(Box::new(fact_ref))
    }
}

pub(super) struct LoadedFactRun {
    pub(super) view: store::VerifiedRunView,
    pub(super) node_id: NodeId,
    pub(super) facts: Vec<ReplayStreamFact>,
    pub(super) fact_refs: Vec<mfm_facts::InternalFactRef>,
    pub(super) response_objects: Vec<(Vec<u8>, StoredArtifactEvidenceRef)>,
    pub(super) exact_requirements: Vec<store::EventArtifactRequirement>,
}

pub(super) fn loaded_fact_run(
    run_discriminator: u8,
    facts: Vec<ReplayStreamFact>,
    query_source: QuerySource,
    include_exact_artifact_pair: bool,
) -> LoadedFactRun {
    let certified = certified_fact_spec();
    let spec = &certified.envelope().spec;
    let spec_hash = certified.spec_hash().clone();
    let node = spec
        .nodes
        .iter()
        .find(|node| {
            node.framework.is_none()
                && node.state_kind == ReplayFactState::kind().expect("fact state kind")
        })
        .cloned()
        .expect("certified fact node");
    let output_cell = spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == node.output_cell)
        .cloned()
        .expect("certified output cell");
    let attempt_id = AttemptId::from_digest(DigestBytes::from_array(
        [run_discriminator.wrapping_add(1); 32],
    ));

    let (admitted, mut objects, run_id) = admitted_run(&certified, run_discriminator);
    let mut records = vec![persisted_record(
        &run_id,
        1,
        1,
        0,
        "admission",
        events::KernelEventPayload::RunAdmitted(Box::new(admitted)),
    )];
    let mut response_objects = Vec::new();
    let mut fact_refs = Vec::new();
    let mut next_sequence = 2;

    if !facts.is_empty() {
        records.push(persisted_record(
            &run_id,
            2,
            2,
            0,
            "fact-attempt-started",
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: spec_hash.clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no: 1,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            }),
        ));

        let descriptor = ReplayStreamFact::descriptor().expect("fact descriptor");
        let descriptor_hash =
            mfm_facts::fact_descriptor_hash(&descriptor).expect("fact descriptor hash");
        for (ordinal, fact) in facts.iter().enumerate() {
            let (claim, bytes, evidence) =
                fact_claim(&descriptor, &descriptor_hash, &node.node_id, fact);
            let envelope = persisted_record(
                &run_id,
                3,
                3,
                ordinal as u32,
                "fact-attempt-terminal",
                events::KernelEventPayload::FactRecorded(events::FactRecorded {
                    spec_hash: spec_hash.clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    claim: claim.clone(),
                }),
            );
            let claim_id = mfm_facts::derive_fact_claim_id(
                run_id.clone(),
                envelope.seq().as_u64(),
                envelope.ordinal().as_u32(),
            )
            .expect("fact claim id");
            let fact_ref = mfm_facts::InternalFactRef::from_claim(
                claim_id,
                envelope.event_id().clone(),
                "1970-01-01T00:00:00Z".to_owned(),
                node.node_id.clone(),
                &claim,
            )
            .expect("internal fact ref");
            objects.insert(object_key(&evidence), (bytes.clone(), evidence.clone()));
            response_objects.push((bytes, evidence));
            fact_refs.push(fact_ref);
            records.push(envelope);
        }

        let output_value = ReplayFactValue { amount: 14 };
        let output_json = serde_json::to_string(&output_value).expect("output json");
        let output_bytes = PlainCanonicalJsonBytes::from_json_str(&output_json)
            .expect("canonical output")
            .to_vec();
        let output_evidence = exact_artifact(
            &output_bytes,
            events::ArtifactRole::StateOutput,
            output_cell.schema_id.clone(),
            spec::MediaType::new("application/json").expect("json media type"),
            Some(output_cell.semantic_type_id.clone()),
            Some(node.node_id.clone()),
        );
        let output_evidence_hash = output_evidence
            .evidence_hash()
            .expect("output evidence hash");
        let terminal_ordinal = u32::try_from(facts.len()).expect("fact count fits ordinal");
        records.push(persisted_record(
            &run_id,
            3,
            3,
            terminal_ordinal,
            "fact-attempt-terminal",
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: spec_hash.clone(),
                node_id: node.node_id.clone(),
                cell_id: output_cell.cell_id.clone(),
                scope_id: output_cell.scope_id.clone(),
                attempt_id: attempt_id.clone(),
                semantic_type_id: output_cell.semantic_type_id.clone(),
                schema_id: output_cell.schema_id.clone(),
                value_lineage: output_cell.value_lineage.clone(),
                context: output_cell.context.clone(),
                artifact_id: output_evidence.artifact_id.clone(),
                content_digest: output_evidence.digest.clone(),
                evidence_hash: output_evidence_hash,
                producer_state_kind: Some(node.state_kind.clone()),
                producer_state_version: Some(node.state_version.clone()),
            }),
        ));
        records.push(persisted_record(
            &run_id,
            3,
            3,
            terminal_ordinal + 1,
            "fact-attempt-terminal",
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: spec_hash.clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                output_cell_id: output_cell.cell_id.clone(),
            }),
        ));
        objects.insert(
            object_key(&output_evidence),
            (output_bytes, output_evidence),
        );
        next_sequence = 4;
    }

    let query_ref = match query_source {
        QuerySource::None => None,
        QuerySource::FirstRecorded => Some(
            fact_refs
                .first()
                .cloned()
                .expect("first-recorded query requires one recorded fact"),
        ),
        QuerySource::External(fact_ref) => Some(*fact_ref),
    };
    if let Some(fact_ref) = query_ref {
        let (query_payload, query_bytes, query_evidence) =
            fact_query_artifact(&spec_hash, &node.node_id, &attempt_id, fact_ref);
        records.push(persisted_record(
            &run_id,
            next_sequence,
            next_sequence,
            0,
            "fact-query-evidence",
            events::KernelEventPayload::ArtifactReferenced(query_payload),
        ));
        objects.insert(object_key(&query_evidence), (query_bytes, query_evidence));
        next_sequence += 1;
    }

    let mut exact_requirements = Vec::new();
    if include_exact_artifact_pair {
        let exact_bytes = br#"{"same":"content"}"#.to_vec();
        let first = exact_artifact(
            &exact_bytes,
            events::ArtifactRole::TypedConfig,
            schema_id("mfm.replay.test.exact_first", 0x91),
            spec::MediaType::new("application/json").expect("json media type"),
            None,
            None,
        );
        let second = exact_artifact(
            &exact_bytes,
            events::ArtifactRole::TypedConfig,
            schema_id("mfm.replay.test.exact_second", 0x92),
            spec::MediaType::new("application/json").expect("json media type"),
            None,
            None,
        );
        assert_eq!(first.artifact_id, second.artifact_id);
        assert_ne!(
            first.evidence_hash().expect("first evidence hash"),
            second.evidence_hash().expect("second evidence hash")
        );
        let commit_key = store::CommitKey::new("exact-artifact-pair").expect("commit key");
        for (ordinal, evidence) in [first, second].into_iter().enumerate() {
            let reference = events::ArtifactReferenced {
                spec_hash: spec_hash.clone(),
                node_id: None,
                attempt_id: None,
                artifact_ref: event_artifact_ref(&evidence),
            };
            exact_requirements.push(store::artifact_referenced_artifact_requirement(&reference));
            records.push(persisted_kernel_event_envelope_with_ordinal_for_test(
                &run_id,
                next_sequence,
                next_sequence,
                ordinal as u32,
                commit_key.clone(),
                events::KernelEventPayload::ArtifactReferenced(reference),
            ));
            objects.insert(object_key(&evidence), (exact_bytes.clone(), evidence));
        }
    }

    let backend = StaticRunJournalBackendForTest::new(run_id.clone(), records, objects);
    let journal = poll_ready_store_future_for_test(backend.load_committed_journal(&run_id))
        .expect("structurally verified journal");
    let view = journal
        .verify(certified)
        .expect("certified fact history view");

    LoadedFactRun {
        view,
        node_id: node.node_id,
        facts,
        fact_refs,
        response_objects,
        exact_requirements,
    }
}

pub(super) fn retained_source_fact(
    view: &store::VerifiedRunView,
    claim_id: &mfm_facts::FactClaimId,
) -> RetainedSourceFactReplayEvent {
    let mut retained = None;
    let flow = store::current_lifecycle::read(view).visit_records(|record| {
        if !matches!(
            record.kind(),
            store::current_lifecycle::CurrentRecordKindRef::FactRecorded(_)
        ) {
            return ControlFlow::Continue(());
        }
        let candidate =
            RetainedSourceFactReplayEvent::from_current_record(record).expect("source fact");
        if candidate.fact_claim_id() == claim_id {
            retained = Some(candidate);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    assert!(matches!(flow, ControlFlow::Break(())));
    retained.expect("retained source fact")
}

pub(super) fn verified_response_object(
    fixture: &LoadedFactRun,
    index: usize,
) -> store::VerifiedRetainedArtifactBytes {
    let (bytes, evidence) = fixture
        .response_objects
        .get(index)
        .expect("response object");
    let requirement =
        store::fact_response_artifact_requirement(fixture.fact_refs.get(index).expect("fact ref"));
    store::VerifiedRetainedArtifactBytes::new(bytes.clone(), evidence.clone(), &requirement)
        .expect("verified response object")
}

fn certified_fact_spec() -> mfm_certify::CertifiedTypedSpec {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<ReplayFactState>()
        .expect("fact state registration");
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root scope"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let input = root.seed(
                mfm_program::SeedKey::new("initial")?,
                CanonicalSeed::from_value(&ReplayFactValue { amount: 7 })?,
            )?;
            let result = root.scope().state::<ReplayFactState, _>(
                StateKey::new("fact-state")?,
                NoContext,
                ReplayFactConfig { multiplier: 2 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &ReplayFactOutputs { result },
            )
        },
    )
    .expect("fact program draft");
    mfm_certify::certify_program_draft(&draft).expect("certified fact program")
}

fn admitted_run(
    certified: &mfm_certify::CertifiedTypedSpec,
    run_discriminator: u8,
) -> (events::RunAdmitted, store::ArtifactByteAuthorityMap, RunId) {
    let spec = &certified.envelope().spec;
    let config_json =
        serde_json::to_string(&ReplayFactConfig { multiplier: 2 }).expect("config json");
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
            let evidence = StoredArtifactEvidenceRef {
                artifact_id: config_ref.artifact_id.clone(),
                digest: config_ref.digest.clone(),
                byte_len: config_ref.byte_len,
                media_type: config_ref.media_type.clone(),
                schema_id: Some(config_ref.schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::TypedConfig,
            };
            (bytes, evidence)
        })
        .collect::<Vec<_>>();

    let seed_spec = spec.seeds.first().cloned().expect("one seed");
    let seed = CanonicalSeed::from_value(&ReplayFactValue { amount: 7 }).expect("canonical seed");
    let seed_bytes = seed.canonical_json().as_bytes().to_vec();
    let seed_evidence = StoredArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(
            seed.content_digest().algorithm(),
            *seed.content_digest().digest(),
        ),
        digest: seed.content_digest().clone(),
        byte_len: seed.byte_len() as u64,
        media_type: spec::MediaType::new("application/json").expect("json media type"),
        schema_id: Some(seed_spec.schema_id.clone()),
        semantic_type_id: Some(seed_spec.semantic_type_id.clone()),
        producer_node_id: None,
        producer_seed_id: Some(seed_spec.seed_id.clone()),
        artifact_role: events::ArtifactRole::SeedInput,
    };

    let descriptor = ReplayStreamFact::descriptor().expect("fact descriptor");
    let descriptor_bytes = mfm_facts::canonical_fact_descriptor_bytes(&descriptor)
        .expect("canonical fact descriptor")
        .to_vec();
    let descriptor_evidence = exact_artifact(
        &descriptor_bytes,
        events::ArtifactRole::FactDescriptor,
        mfm_facts::fact_descriptor_schema_id().expect("descriptor schema"),
        spec::MediaType::new("application/json").expect("json media type"),
        None,
        None,
    );

    let persisted = certified
        .to_persisted_parts()
        .expect("persisted certified authority");
    let spec_evidence = exact_artifact(
        persisted.spec_bytes(),
        events::ArtifactRole::TypedExecutionSpec,
        spec::typed_execution_spec_schema_id().expect("typed spec schema"),
        spec.media_type.clone(),
        None,
        None,
    );
    let certificate_evidence = exact_artifact(
        persisted.certificate_bytes(),
        events::ArtifactRole::TypedSpecCertificate,
        mfm_certify::typed_spec_certificate_schema_id().expect("certificate schema"),
        spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).expect("certificate media type"),
        None,
        None,
    );
    let identity_material = run_identity_material_for_test(
        certified.spec_hash().clone(),
        &format!("{run_discriminator:02x}").repeat(16),
    );
    let run_id = identity_material.derive_run_id().expect("run id");
    let admitted = events::RunAdmitted {
        run_id: run_id.clone(),
        identity_material,
        entry_point: events::EntryPointLaunchEvidence::new(
            "mfm.replay.test/fact_fixture@1",
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
        fact_descriptor_artifacts: vec![run_artifact_ref_from_store_artifact_for_test(
            &descriptor_evidence,
        )],
        spec_version: spec.spec_version.clone(),
        lowering_version: spec.lowering_version.clone(),
        public_output_schema_id: spec.public_outputs.public_schema_id.clone(),
        saga_policy_digest: spec.saga.saga_policy_digest().expect("saga policy digest"),
        descriptor_identities: spec.descriptor_identities.clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        capability_implementations: Vec::new(),
        admitted_binding_digest: fixed_content_digest_for_test(0x24),
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
                evidence_hash: seed_evidence.evidence_hash().expect("seed evidence hash"),
                byte_len: seed_evidence.byte_len,
                media_type: seed_evidence.media_type.clone(),
            },
        }],
    };

    let mut objects = store::ArtifactByteAuthorityMap::from([
        (
            object_key(&spec_evidence),
            (persisted.spec_bytes().to_vec(), spec_evidence),
        ),
        (
            object_key(&certificate_evidence),
            (persisted.certificate_bytes().to_vec(), certificate_evidence),
        ),
        (object_key(&seed_evidence), (seed_bytes, seed_evidence)),
        (
            object_key(&descriptor_evidence),
            (descriptor_bytes, descriptor_evidence),
        ),
    ]);
    for (bytes, evidence) in config_objects {
        objects.insert(object_key(&evidence), (bytes, evidence));
    }
    (admitted, objects, run_id)
}

fn fact_claim(
    descriptor: &mfm_facts::FactDescriptor,
    descriptor_hash: &ContentDigest,
    node_id: &NodeId,
    fact: &ReplayStreamFact,
) -> (mfm_facts::FactClaim, Vec<u8>, StoredArtifactEvidenceRef) {
    let subject_json = serde_json::to_value(fact.subject()).expect("fact subject json");
    let subject = mfm_facts::typed_fact_subject_evidence(descriptor, &subject_json)
        .expect("fact subject evidence");
    let response_json = serde_json::to_string(fact.response()).expect("fact response json");
    let response_bytes = PlainCanonicalJsonBytes::from_json_str(&response_json)
        .expect("canonical fact response")
        .to_vec();
    let response_evidence = exact_artifact(
        &response_bytes,
        events::ArtifactRole::FactResponse,
        descriptor.response_schema_id().clone(),
        spec::MediaType::new("application/json").expect("json media type"),
        None,
        Some(node_id.clone()),
    );
    let claim = mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        fact_kind: descriptor.fact_kind().clone(),
        fact_descriptor_hash: descriptor_hash.clone(),
        subject,
        response: mfm_facts::FactResponseEvidence::new(
            descriptor.response_schema_id().clone(),
            response_evidence.digest.clone(),
            response_evidence.artifact_id.clone(),
            response_evidence
                .evidence_hash()
                .expect("response evidence hash"),
        ),
    })
    .expect("fact claim");
    (claim, response_bytes, response_evidence)
}

fn fact_query_artifact(
    spec_hash: &SpecHash,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    fact_ref: mfm_facts::InternalFactRef,
) -> (
    events::ArtifactReferenced,
    Vec<u8>,
    StoredArtifactEvidenceRef,
) {
    let descriptor = ReplayStreamFact::descriptor().expect("fact descriptor");
    let input = mfm_facts::FactQueryInput::new(
        Vec::new(),
        vec![FactFieldId::new("result.amount").expect("return field")],
        FactOrderingName::new("result.amount.asc").expect("ordering"),
        Some(1),
    )
    .expect("query input");
    let plan = mfm_facts::compile_fact_query_plan(&descriptor, input).expect("query plan");
    let rows = [mfm_facts::FactQueryResultRow::new(fact_ref, Vec::new())];
    let receipt = mfm_facts::FactQueryReceipt::from_rows(
        mfm_facts::StoreReadFrontier::new(
            StoreScopeId::new(format!("mfm.store_scope.v1:{}", "ab".repeat(16)))
                .expect("store scope"),
            mfm_facts::StoreCommitOrder::new(3),
        ),
        &rows,
        false,
        Some(1),
    )
    .expect("fact query receipt");
    let selection =
        mfm_facts::FactSelectionEvidence::new(fixed_content_digest_for_test(0x25), vec![0], None)
            .expect("fact selection");
    let evidence = mfm_facts::FactQueryEvidence::new(plan, receipt, selection);
    let bytes = mfm_facts::canonical_fact_query_evidence_bytes(&evidence)
        .expect("canonical query evidence")
        .to_vec();
    let artifact = exact_artifact(
        &bytes,
        events::ArtifactRole::FactQueryEvidence,
        mfm_facts::fact_query_evidence_schema_id().expect("query evidence schema"),
        spec::MediaType::new("application/json").expect("json media type"),
        None,
        Some(node_id.clone()),
    );
    let payload = events::ArtifactReferenced {
        spec_hash: spec_hash.clone(),
        node_id: Some(node_id.clone()),
        attempt_id: Some(attempt_id.clone()),
        artifact_ref: event_artifact_ref(&artifact),
    };
    (payload, bytes, artifact)
}

fn exact_artifact(
    bytes: &[u8],
    role: events::ArtifactRole,
    schema_id: SchemaId,
    media_type: spec::MediaType,
    semantic_type_id: Option<SemanticTypeId>,
    producer_node_id: Option<NodeId>,
) -> StoredArtifactEvidenceRef {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    StoredArtifactEvidenceRef {
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

fn event_artifact_ref(evidence: &StoredArtifactEvidenceRef) -> events::ArtifactEvidenceRef {
    events::ArtifactEvidenceRef {
        artifact_id: evidence.artifact_id.clone(),
        role: evidence.artifact_role,
        schema_id: evidence.schema_id.clone().expect("schema-bearing fixture"),
        semantic_type_id: evidence.semantic_type_id.clone(),
        content_digest: evidence.digest.clone(),
        evidence_hash: evidence.evidence_hash().expect("artifact evidence hash"),
        byte_len: evidence.byte_len,
        media_type: evidence.media_type.clone(),
    }
}

fn object_key(evidence: &StoredArtifactEvidenceRef) -> (ArtifactId, ContentDigest) {
    (
        evidence.artifact_id.clone(),
        evidence.evidence_hash().expect("artifact evidence hash"),
    )
}

fn persisted_record(
    run_id: &RunId,
    sequence: u64,
    store_commit_order: u64,
    ordinal: u32,
    commit_key: &str,
    payload: events::KernelEventPayload,
) -> store::KernelEventEnvelope {
    if ordinal == 0 {
        return persisted_kernel_event_envelope_for_test(
            run_id,
            sequence,
            store_commit_order,
            store::CommitKey::new(commit_key).expect("commit key"),
            payload,
        );
    }
    persisted_kernel_event_envelope_with_ordinal_for_test(
        run_id,
        sequence,
        store_commit_order,
        ordinal,
        store::CommitKey::new(commit_key).expect("commit key"),
        payload,
    )
}

fn schema_id(name: &str, discriminator: u8) -> SchemaId {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([discriminator; 32]),
    )
    .expect("schema id")
}
