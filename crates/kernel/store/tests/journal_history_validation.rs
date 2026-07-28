#![cfg(feature = "test-support")]

use std::collections::BTreeMap;
use std::ops::ControlFlow;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilitySpec, ReadExternal, ReadExternalRole};
use mfm_events::v1::{self as events, ArtifactRole, KernelEventPayload};
use mfm_facts::MfmFactType as _;
use mfm_ids::{
    ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, ContentDigest, ContextRef,
    ContextResourceKind, ContextStage, DigestAlgorithm, DigestBytes, NodeId, RunId, SchemaId,
    SemanticTypeId, StateKind, StateVersion,
};
use mfm_program::{
    build_root_with_registries, AdapterBindingSpec, CanonicalSeed, ExternalReadEvidenceSet,
    NoContext, PublicOutputKey, ReadState, RootBuilder, ScopeKey, StateKey, StateRegistryBuilder,
    StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmFactType, MfmValue, PublicOutputs};
use mfm_spec::v1::{self as spec, MediaType};
use mfm_store::v1::test_support::{
    fixed_content_digest_for_test, persisted_kernel_event_envelope_for_test,
    persisted_kernel_event_envelope_with_ordinal_for_test, poll_ready_store_future_for_test,
    run_artifact_ref_from_store_artifact_for_test, run_identity_material_for_test,
    StaticRunJournalBackendForTest,
};
use mfm_store::v1::{
    ArtifactByteAuthorityMap, ArtifactEvidenceRef, CommitKey, CommittedRunJournal,
    KernelEventEnvelope, RunJournalStore as _, StoreError, VerifiedRunView,
};
use serde::{Deserialize, Serialize};

#[path = "journal_history_validation/side_effects.rs"]
mod side_effects;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.store.test",
    name = "history_value",
    version = "1",
    schema = "mfm.store.test.history_value"
)]
struct HistoryValue {
    amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.store.test",
    name = "history_subject",
    version = "1",
    schema = "mfm.store.test.history_subject"
)]
struct HistorySubject {
    account: String,
}

#[allow(clippy::duplicated_attributes)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.store.test",
    name = "history_fact",
    version = "1",
    schema = "mfm.store.test.history_fact"
)]
#[mfm_fact(kind = "mfm.store.test.history")]
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
    operator = "equal",
    exposure = "returnable",
    sortable
))]
#[mfm_fact(ordering(
    name = "result.amount.asc",
    term(field = "result.amount", direction = "ascending", nulls = "last")
))]
struct HistoryFact {
    subject: HistorySubject,
    response: HistoryValue,
}

impl HistoryFact {
    fn new(account: &str, amount: u64) -> Self {
        Self {
            subject: HistorySubject {
                account: account.to_owned(),
            },
            response: HistoryValue { amount },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.store.test",
    name = "alternate_history_subject",
    version = "1",
    schema = "mfm.store.test.alternate_history_subject"
)]
struct AlternateHistorySubject {
    account: String,
}

#[allow(clippy::duplicated_attributes)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.store.test",
    name = "alternate_history_fact",
    version = "1",
    schema = "mfm.store.test.alternate_history_fact"
)]
#[mfm_fact(kind = "mfm.store.test.alternate_history")]
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
    operator = "equal",
    exposure = "returnable"
))]
struct AlternateHistoryFact {
    subject: AlternateHistorySubject,
    response: HistoryValue,
}

impl AlternateHistoryFact {
    fn new(account: &str, amount: u64) -> Self {
        Self {
            subject: AlternateHistorySubject {
                account: account.to_owned(),
            },
            response: HistoryValue { amount },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct HistoryConfig {
    multiplier: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.store.test.history_outputs")]
struct HistoryOutputs<'program, 'scope> {
    result: mfm_program::Handle<'program, 'scope, HistoryValue>,
}

struct HistoryReadCapability;

impl CapabilitySpec for HistoryReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.store.test",
            "history_read",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x61; 32]),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.store.test.history_read.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.store.test.history_read"
    }
}

struct HistoryReadState {
    config: HistoryConfig,
}

impl StateSpec for HistoryReadState {
    type Config = HistoryConfig;
    type Context = NoContext;
    type Input = HistoryValue;
    type Output = HistoryValue;
    type Effect = ReadExternal;
    type Caps = (HistoryReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.store.test",
            "history_read_state",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x62; 32]),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.store.test.history_read_state.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.store.test.history_read_state"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        Ok(vec![AdapterBindingSpec {
            adapter_kind: mfm_ids::AdapterKind::new(
                "mfm.store.test",
                "history_adapter",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x63; 32]),
            )
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            adapter_version: mfm_ids::AdapterVersion::new("mfm.store.test.history_adapter.v1")
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
        }])
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for HistoryReadState {
    type Plan = HistoryValue;
    type Evidence = HistoryValue;
    type Facts = mfm_values::NonEmpty<HistoryFact>;

    fn plan(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        Ok(HistoryValue {
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
                "history fixture evidence mismatch".to_owned(),
            ));
        }
        Ok((
            output,
            mfm_values::NonEmpty::new(HistoryFact::new("primary", 1), Vec::new()),
        ))
    }
}

struct AlternateHistoryReadState {
    config: HistoryConfig,
}

impl StateSpec for AlternateHistoryReadState {
    type Config = HistoryConfig;
    type Context = NoContext;
    type Input = HistoryValue;
    type Output = HistoryValue;
    type Effect = ReadExternal;
    type Caps = (HistoryReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.store.test",
            "alternate_history_read_state",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x64; 32]),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.store.test.alternate_history_read_state.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.store.test.alternate_history_read_state"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        HistoryReadState::adapter_bindings()
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for AlternateHistoryReadState {
    type Plan = HistoryValue;
    type Evidence = HistoryValue;
    type Facts = mfm_values::NonEmpty<AlternateHistoryFact>;

    fn plan(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        Ok(HistoryValue {
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
                "alternate history fixture evidence mismatch".to_owned(),
            ));
        }
        Ok((
            output,
            mfm_values::NonEmpty::new(AlternateHistoryFact::new("alternate", 1), Vec::new()),
        ))
    }
}

struct HistoryFixture {
    certified: mfm_certify::CertifiedTypedSpec,
    run_id: RunId,
    spec_hash: mfm_ids::SpecHash,
    node: spec::NodeSpec,
    alternate_node: spec::NodeSpec,
    alternate_output_cell: spec::CellSpec,
    records: Vec<KernelEventEnvelope>,
    objects: ArtifactByteAuthorityMap,
}

fn certified_history_spec() -> mfm_certify::CertifiedTypedSpec {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<HistoryReadState>()
        .expect("primary state registration");
    states
        .register::<AlternateHistoryReadState>()
        .expect("alternate state registration");
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root scope"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let input = root.seed(
                mfm_program::SeedKey::new("initial")?,
                CanonicalSeed::from_value(&HistoryValue { amount: 7 })?,
            )?;
            let result = root.scope().state::<HistoryReadState, _>(
                StateKey::new("history-state")?,
                NoContext,
                HistoryConfig { multiplier: 2 },
                input,
            )?;
            let alternate = root.scope().state::<AlternateHistoryReadState, _>(
                StateKey::new("alternate-history-state")?,
                NoContext,
                HistoryConfig { multiplier: 2 },
                result.clone(),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &HistoryOutputs { result: alternate },
            )
        },
    )
    .expect("history program draft");
    mfm_certify::certify_program_draft(&draft).expect("certified history program")
}

fn history_fixture(facts: &[HistoryFact]) -> HistoryFixture {
    let certified = certified_history_spec();
    let spec = &certified.envelope().spec;
    let spec_hash = certified.spec_hash().clone();
    let node = spec
        .nodes
        .iter()
        .find(|node| {
            node.framework.is_none()
                && node.state_kind == HistoryReadState::kind().expect("history state kind")
        })
        .cloned()
        .expect("certified history node");
    let output_cell = spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == node.output_cell)
        .cloned()
        .expect("certified history output cell");
    let alternate_node = spec
        .nodes
        .iter()
        .find(|node| {
            node.framework.is_none()
                && node.state_kind
                    == AlternateHistoryReadState::kind().expect("alternate history state kind")
        })
        .cloned()
        .expect("certified alternate history node");
    let alternate_output_cell = spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == alternate_node.output_cell)
        .cloned()
        .expect("certified alternate history output cell");
    let attempt_id = attempt_id(0x66);
    let (admitted, mut objects, run_id) = admitted_run(&certified);
    let mut records = vec![persisted_record(
        &run_id,
        1,
        1,
        0,
        "history-admission",
        KernelEventPayload::RunAdmitted(Box::new(admitted)),
    )];
    records.push(persisted_record(
        &run_id,
        2,
        2,
        0,
        "history-attempt-start",
        KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: spec_hash.clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            attempt_no: 1,
            state_kind: node.state_kind.clone(),
            state_version: node.state_version.clone(),
        }),
    ));

    let descriptor = HistoryFact::descriptor().expect("history fact descriptor");
    let descriptor_hash =
        mfm_facts::fact_descriptor_hash(&descriptor).expect("history descriptor hash");
    for (ordinal, fact) in facts.iter().enumerate() {
        let (claim, bytes, evidence) =
            fact_claim(&descriptor, &descriptor_hash, &node.node_id, fact);
        records.push(persisted_record(
            &run_id,
            3,
            3,
            ordinal as u32,
            "history-attempt-terminal",
            KernelEventPayload::FactRecorded(events::FactRecorded {
                spec_hash: spec_hash.clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                claim,
            }),
        ));
        objects.insert(object_key(&evidence), (bytes, evidence));
    }

    let output_value = HistoryValue { amount: 14 };
    let output_json = serde_json::to_string(&output_value).expect("output json");
    let output_bytes = PlainCanonicalJsonBytes::from_json_str(&output_json)
        .expect("canonical output")
        .to_vec();
    let output_evidence = exact_artifact(
        &output_bytes,
        ArtifactRole::StateOutput,
        output_cell.schema_id.clone(),
        MediaType::new("application/json").expect("json media type"),
        Some(output_cell.semantic_type_id.clone()),
        Some(node.node_id.clone()),
    );
    let terminal_ordinal = u32::try_from(facts.len()).expect("fact count fits ordinal");
    records.push(persisted_record(
        &run_id,
        3,
        3,
        terminal_ordinal,
        "history-attempt-terminal",
        KernelEventPayload::CellProduced(events::CellProduced {
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
            evidence_hash: output_evidence
                .evidence_hash()
                .expect("output evidence hash"),
            producer_state_kind: Some(node.state_kind.clone()),
            producer_state_version: Some(node.state_version.clone()),
        }),
    ));
    records.push(persisted_record(
        &run_id,
        3,
        3,
        terminal_ordinal + 1,
        "history-attempt-terminal",
        KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
            spec_hash: spec_hash.clone(),
            node_id: node.node_id.clone(),
            attempt_id,
            output_cell_id: output_cell.cell_id.clone(),
        }),
    ));
    objects.insert(
        object_key(&output_evidence),
        (output_bytes, output_evidence),
    );

    HistoryFixture {
        certified,
        run_id,
        spec_hash,
        node,
        alternate_node,
        alternate_output_cell,
        records,
        objects,
    }
}

fn append_alternate_fact_settlement(fixture: &mut HistoryFixture, fact: &AlternateHistoryFact) {
    let attempt_id = attempt_id(0x68);
    fixture.records.push(persisted_record(
        &fixture.run_id,
        4,
        4,
        0,
        "alternate-attempt-start",
        KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: fixture.spec_hash.clone(),
            node_id: fixture.alternate_node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            attempt_no: 1,
            state_kind: fixture.alternate_node.state_kind.clone(),
            state_version: fixture.alternate_node.state_version.clone(),
        }),
    ));
    let descriptor = AlternateHistoryFact::descriptor().expect("alternate descriptor");
    let descriptor_hash =
        mfm_facts::fact_descriptor_hash(&descriptor).expect("alternate descriptor hash");
    let (claim, response_bytes, response_evidence) = fact_claim(
        &descriptor,
        &descriptor_hash,
        &fixture.alternate_node.node_id,
        fact,
    );
    fixture.records.push(persisted_record(
        &fixture.run_id,
        5,
        5,
        0,
        "alternate-attempt-terminal",
        KernelEventPayload::FactRecorded(events::FactRecorded {
            spec_hash: fixture.spec_hash.clone(),
            node_id: fixture.alternate_node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            claim,
        }),
    ));
    fixture.objects.insert(
        object_key(&response_evidence),
        (response_bytes, response_evidence),
    );

    let output_json =
        serde_json::to_string(&HistoryValue { amount: 28 }).expect("alternate output json");
    let output_bytes = PlainCanonicalJsonBytes::from_json_str(&output_json)
        .expect("canonical alternate output")
        .to_vec();
    let output_evidence = exact_artifact(
        &output_bytes,
        ArtifactRole::StateOutput,
        fixture.alternate_output_cell.schema_id.clone(),
        MediaType::new("application/json").expect("json media type"),
        Some(fixture.alternate_output_cell.semantic_type_id.clone()),
        Some(fixture.alternate_node.node_id.clone()),
    );
    fixture.records.push(persisted_record(
        &fixture.run_id,
        5,
        5,
        1,
        "alternate-attempt-terminal",
        KernelEventPayload::CellProduced(events::CellProduced {
            spec_hash: fixture.spec_hash.clone(),
            node_id: fixture.alternate_node.node_id.clone(),
            cell_id: fixture.alternate_output_cell.cell_id.clone(),
            scope_id: fixture.alternate_output_cell.scope_id.clone(),
            attempt_id: attempt_id.clone(),
            semantic_type_id: fixture.alternate_output_cell.semantic_type_id.clone(),
            schema_id: fixture.alternate_output_cell.schema_id.clone(),
            value_lineage: fixture.alternate_output_cell.value_lineage.clone(),
            context: fixture.alternate_output_cell.context.clone(),
            artifact_id: output_evidence.artifact_id.clone(),
            content_digest: output_evidence.digest.clone(),
            evidence_hash: output_evidence
                .evidence_hash()
                .expect("alternate output evidence hash"),
            producer_state_kind: Some(fixture.alternate_node.state_kind.clone()),
            producer_state_version: Some(fixture.alternate_node.state_version.clone()),
        }),
    ));
    fixture.records.push(persisted_record(
        &fixture.run_id,
        5,
        5,
        2,
        "alternate-attempt-terminal",
        KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
            spec_hash: fixture.spec_hash.clone(),
            node_id: fixture.alternate_node.node_id.clone(),
            attempt_id,
            output_cell_id: fixture.alternate_output_cell.cell_id.clone(),
        }),
    ));
    fixture.objects.insert(
        object_key(&output_evidence),
        (output_bytes, output_evidence),
    );
}

fn admitted_run(
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

    let seed_spec = spec.seeds.first().cloned().expect("one seed");
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

    let descriptors = [
        HistoryFact::descriptor().expect("history fact descriptor"),
        AlternateHistoryFact::descriptor().expect("alternate history fact descriptor"),
    ]
    .into_iter()
    .map(|descriptor| {
        let bytes = mfm_facts::canonical_fact_descriptor_bytes(&descriptor)
            .expect("canonical fact descriptor")
            .to_vec();
        let evidence = exact_artifact(
            &bytes,
            ArtifactRole::FactDescriptor,
            mfm_facts::fact_descriptor_schema_id().expect("descriptor schema"),
            MediaType::new("application/json").expect("json media type"),
            None,
            None,
        );
        (bytes, evidence)
    })
    .collect::<Vec<_>>();

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
        run_identity_material_for_test(certified.spec_hash().clone(), &"65".repeat(16));
    let run_id = identity_material.derive_run_id().expect("run id");
    let admitted = events::RunAdmitted {
        run_id: run_id.clone(),
        identity_material,
        entry_point: events::EntryPointLaunchEvidence::new(
            "mfm.store.test/history_fixture@1",
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
        fact_descriptor_artifacts: descriptors
            .iter()
            .map(|(_, evidence)| run_artifact_ref_from_store_artifact_for_test(evidence))
            .collect(),
        spec_version: spec.spec_version.clone(),
        lowering_version: spec.lowering_version.clone(),
        public_output_schema_id: spec.public_outputs.public_schema_id.clone(),
        saga_policy_digest: spec.saga.saga_policy_digest().expect("saga policy digest"),
        descriptor_identities: spec.descriptor_identities.clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        capability_implementations: Vec::new(),
        admitted_binding_digest: fixed_content_digest_for_test(0x66),
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
    for (bytes, evidence) in config_objects.into_iter().chain(descriptors) {
        objects.insert(object_key(&evidence), (bytes, evidence));
    }
    (admitted, objects, run_id)
}

fn fact_claim<F>(
    descriptor: &mfm_facts::FactDescriptor,
    descriptor_hash: &ContentDigest,
    node_id: &NodeId,
    fact: &F,
) -> (mfm_facts::FactClaim, Vec<u8>, ArtifactEvidenceRef)
where
    F: mfm_facts::MfmFactType,
{
    let subject_json = serde_json::to_value(fact.subject()).expect("fact subject json");
    let subject = mfm_facts::typed_fact_subject_evidence(descriptor, &subject_json)
        .expect("fact subject evidence");
    let response_json = serde_json::to_string(fact.response()).expect("fact response json");
    let response_bytes = PlainCanonicalJsonBytes::from_json_str(&response_json)
        .expect("canonical fact response")
        .to_vec();
    let response_evidence = exact_artifact(
        &response_bytes,
        ArtifactRole::FactResponse,
        descriptor.response_schema_id().clone(),
        MediaType::new("application/json").expect("json media type"),
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

fn persisted_record(
    run_id: &RunId,
    sequence: u64,
    store_commit_order: u64,
    ordinal: u32,
    commit_key: &str,
    payload: KernelEventPayload,
) -> KernelEventEnvelope {
    if ordinal == 0 {
        return persisted_kernel_event_envelope_for_test(
            run_id,
            sequence,
            store_commit_order,
            CommitKey::new(commit_key).expect("commit key"),
            payload,
        );
    }
    persisted_kernel_event_envelope_with_ordinal_for_test(
        run_id,
        sequence,
        store_commit_order,
        ordinal,
        CommitKey::new(commit_key).expect("commit key"),
        payload,
    )
}

fn replace_record_payload(
    record: &KernelEventEnvelope,
    payload: KernelEventPayload,
) -> KernelEventEnvelope {
    persisted_kernel_event_envelope_with_ordinal_for_test(
        record.run_id(),
        record.seq().as_u64(),
        record.store_commit_order().as_u64(),
        record.ordinal().as_u32(),
        record.commit_key().clone(),
        payload,
    )
}

fn attempt_id(discriminator: u8) -> AttemptId {
    AttemptId::from_digest(DigestBytes::from_array([discriminator; 32]))
}

fn load_journal(
    run_id: &RunId,
    records: Vec<KernelEventEnvelope>,
    objects: ArtifactByteAuthorityMap,
) -> Result<CommittedRunJournal, StoreError> {
    let backend = StaticRunJournalBackendForTest::new(run_id.clone(), records, objects);
    poll_ready_store_future_for_test(backend.load_committed_journal(run_id))
}

fn verify_fixture(fixture: HistoryFixture) -> Result<VerifiedRunView, StoreError> {
    load_journal(&fixture.run_id, fixture.records, fixture.objects)?.verify(fixture.certified)
}

fn assert_certified_history_rejects(fixture: HistoryFixture, expected: &str) {
    let journal = load_journal(&fixture.run_id, fixture.records, fixture.objects)
        .expect("mutation remains a structurally loadable committed journal");
    let error = journal
        .verify(fixture.certified)
        .expect_err("certified history mutation must reject");
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

fn assert_current_projection_rejects(fixture: HistoryFixture, expected: &str) {
    let journal = load_journal(&fixture.run_id, fixture.records, fixture.objects)
        .expect("mutation remains a structurally loadable committed journal");
    let error = journal
        .verify(fixture.certified)
        .expect_err("current semantic projection mutation must reject");
    assert!(
        matches!(
            &error,
            StoreError::ProjectionConflict { message, .. } if message.contains(expected)
        ),
        "unexpected error: {error:?}"
    );
}

fn mismatched_context(node: &spec::NodeSpec) -> spec::CellContextSpec {
    spec::CellContextSpec::Bound {
        context_ref: ContextRef::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x67; 32]),
        ),
        resource_kind: ContextResourceKind::new("mfm.store.test.unrelated_context")
            .expect("context resource kind"),
        stage: ContextStage::new("unrelated").expect("context stage"),
        producer: Box::new(spec::ContextProducerSpec {
            producer_descriptor_ids: vec![node.descriptor_id.clone()],
            seed_producers_allowed: false,
        }),
    }
}

#[test]
fn certified_history_accepts_exact_ordered_fact_settlement() {
    let fixture = history_fixture(&[
        HistoryFact::new("account-a", 41),
        HistoryFact::new("account-b", 42),
    ]);
    let view = verify_fixture(fixture).expect("valid fact history");
    assert_eq!(view.current_run_sequence(), Some(3));
}

#[test]
fn certified_history_rejects_produced_cell_context_mismatch() {
    let mut fixture = history_fixture(&[HistoryFact::new("account-a", 41)]);
    let context = mismatched_context(&fixture.node);
    let produced_index = fixture
        .records
        .iter()
        .position(|record| matches!(record.payload(), KernelEventPayload::CellProduced(_)))
        .expect("produced cell");
    let mut produced = match fixture.records[produced_index].payload().clone() {
        KernelEventPayload::CellProduced(payload) => payload,
        _ => unreachable!("located produced cell"),
    };
    produced.context = context;
    fixture.records[produced_index] = replace_record_payload(
        &fixture.records[produced_index],
        KernelEventPayload::CellProduced(produced),
    );
    assert_certified_history_rejects(fixture, "does not match certified metadata");
}

#[test]
fn certified_history_rejects_fact_after_terminal_cell() {
    let mut fixture = history_fixture(&[HistoryFact::new("account-a", 41)]);
    let fact_index = fixture
        .records
        .iter()
        .position(|record| matches!(record.payload(), KernelEventPayload::FactRecorded(_)))
        .expect("fact record");
    let cell_index = fixture
        .records
        .iter()
        .position(|record| matches!(record.payload(), KernelEventPayload::CellProduced(_)))
        .expect("cell record");
    let fact_payload = fixture.records[fact_index].payload().clone();
    let cell_payload = fixture.records[cell_index].payload().clone();
    fixture.records[fact_index] =
        replace_record_payload(&fixture.records[fact_index], cell_payload);
    fixture.records[cell_index] =
        replace_record_payload(&fixture.records[cell_index], fact_payload);
    assert_current_projection_rejects(fixture, "facts, produced cell, then completion");
}

#[test]
fn certified_history_rejects_duplicate_fact_key_in_one_settlement() {
    let duplicate = HistoryFact::new("account-a", 41);
    let fixture = history_fixture(&[duplicate.clone(), duplicate]);
    assert_current_projection_rejects(fixture, "fact keys must be unique");
}

#[test]
fn certified_history_rejects_descriptor_certified_only_for_another_node() {
    let mut fixture = history_fixture(&[HistoryFact::new("account-a", 41)]);
    let descriptor = AlternateHistoryFact::descriptor().expect("alternate descriptor");
    let descriptor_hash =
        mfm_facts::fact_descriptor_hash(&descriptor).expect("alternate descriptor hash");
    let alternate = AlternateHistoryFact::new("account-a", 41);
    let (claim, bytes, evidence) = fact_claim(
        &descriptor,
        &descriptor_hash,
        &fixture.node.node_id,
        &alternate,
    );
    let fact_index = fixture
        .records
        .iter()
        .position(|record| matches!(record.payload(), KernelEventPayload::FactRecorded(_)))
        .expect("fact record");
    let mut fact = match fixture.records[fact_index].payload().clone() {
        KernelEventPayload::FactRecorded(payload) => payload,
        _ => unreachable!("located fact record"),
    };
    fact.claim = claim;
    fixture.records[fact_index] = replace_record_payload(
        &fixture.records[fact_index],
        KernelEventPayload::FactRecorded(fact),
    );
    fixture
        .objects
        .insert(object_key(&evidence), (bytes, evidence));
    assert_certified_history_rejects(fixture, "is not certified for node");
}

#[test]
fn fact_successor_uses_descriptor_authority_retained_by_the_prefix() {
    let fixture = history_fixture(&[HistoryFact::new("account-a", 41)]);
    let initial_records = fixture.records[..1].to_vec();
    let initial = load_journal(&fixture.run_id, initial_records, fixture.objects.clone())
        .expect("admission-only journal");
    let view = initial
        .verify(certified_history_spec())
        .expect("admission-only certified view");
    let successor = load_journal(&fixture.run_id, fixture.records, fixture.objects)
        .expect("successor with fact settlement");

    let advanced = view
        .verify_successor(successor)
        .expect("fact descriptor admitted in prefix remains sufficient");
    assert_eq!(advanced.current_run_sequence(), Some(3));
}

fn assert_fact_projection_equivalent(
    incremental: &VerifiedRunView,
    full: &VerifiedRunView,
    fixture: &HistoryFixture,
) {
    let incremental = mfm_store::v1::current_lifecycle::read(incremental);
    let full = mfm_store::v1::current_lifecycle::read(full);
    for claim_id in [
        mfm_facts::FactClaimId::new(fixture.run_id.clone(), 3, 0).expect("first fact claim id"),
        mfm_facts::FactClaimId::new(fixture.run_id.clone(), 5, 0).expect("alternate fact claim id"),
    ] {
        let incremental_query = incremental
            .fact_query_entry(&claim_id)
            .expect("incremental fact projection");
        let full_query = full
            .fact_query_entry(&claim_id)
            .expect("full fact projection");
        assert_eq!(
            incremental_query
                .internal_ref()
                .expect("incremental fact ref"),
            full_query.internal_ref().expect("full fact ref")
        );
        assert_eq!(
            incremental_query.response_artifact_evidence(),
            full_query.response_artifact_evidence()
        );
        let mut incremental_terms = Vec::new();
        let incremental_flow = incremental_query.visit_terms(|term| {
            incremental_terms.push(term.clone());
            ControlFlow::<()>::Continue(())
        });
        let mut full_terms = Vec::new();
        let full_flow = full_query.visit_terms(|term| {
            full_terms.push(term.clone());
            ControlFlow::<()>::Continue(())
        });
        assert!(matches!(incremental_flow, ControlFlow::Continue(())));
        assert!(matches!(full_flow, ControlFlow::Continue(())));
        assert_eq!(incremental_terms, full_terms);
    }

    for cell_id in [
        &fixture.node.output_cell,
        &fixture.alternate_output_cell.cell_id,
    ] {
        let incremental_cell = incremental
            .cell(cell_id)
            .expect("incremental produced cell")
            .produced()
            .expect("incremental cell is produced");
        let full_cell = full
            .cell(cell_id)
            .expect("full produced cell")
            .produced()
            .expect("full cell is produced");
        assert_eq!(incremental_cell.node_id(), full_cell.node_id());
        assert_eq!(incremental_cell.attempt_id(), full_cell.attempt_id());
        assert_eq!(incremental_cell.schema_id(), full_cell.schema_id());
        assert_eq!(
            incremental_cell.semantic_type_id(),
            full_cell.semantic_type_id()
        );
        assert_eq!(incremental_cell.artifact_id(), full_cell.artifact_id());
        assert_eq!(
            incremental_cell.content_digest(),
            full_cell.content_digest()
        );
        assert_eq!(incremental_cell.evidence_hash(), full_cell.evidence_hash());
    }

    for (node_id, attempt_id) in [
        (&fixture.node.node_id, attempt_id(0x66)),
        (&fixture.alternate_node.node_id, attempt_id(0x68)),
    ] {
        let incremental_attempt = incremental
            .attempt(node_id, &attempt_id)
            .expect("incremental attempt projection");
        let full_attempt = full
            .attempt(node_id, &attempt_id)
            .expect("full attempt projection");
        let incremental_output = match incremental_attempt.status() {
            mfm_store::v1::current_lifecycle::CurrentAttemptStatusRef::Completed {
                output_cell_id,
            } => output_cell_id,
            status => panic!("unexpected incremental attempt status: {status:?}"),
        };
        let full_output = match full_attempt.status() {
            mfm_store::v1::current_lifecycle::CurrentAttemptStatusRef::Completed {
                output_cell_id,
            } => output_cell_id,
            status => panic!("unexpected full attempt status: {status:?}"),
        };
        assert_eq!(incremental_output, full_output);
    }
}

#[test]
fn consecutive_fact_successors_reuse_prefix_descriptors_without_recertification() {
    let mut fixture = history_fixture(&[HistoryFact::new("account-a", 41)]);
    let first_successor_records = fixture.records.clone();
    append_alternate_fact_settlement(&mut fixture, &AlternateHistoryFact::new("account-b", 84));
    let final_records = fixture.records.clone();
    let final_objects = fixture.objects.clone();

    let initial = load_journal(
        &fixture.run_id,
        fixture.records[..1].to_vec(),
        fixture.objects.clone(),
    )
    .expect("admission-only journal");
    let view = initial
        .verify(certified_history_spec())
        .expect("admission-only certified view");
    let first_successor = load_journal(
        &fixture.run_id,
        first_successor_records,
        fixture.objects.clone(),
    )
    .expect("first fact successor");
    let once_advanced = view
        .verify_successor(first_successor)
        .expect("first suffix applies without replaying admission");
    let second_successor = load_journal(
        &fixture.run_id,
        final_records.clone(),
        fixture.objects.clone(),
    )
    .expect("second fact successor");
    let incrementally_advanced = once_advanced
        .verify_successor(second_successor)
        .expect("second suffix reuses prefix descriptors without replaying prior facts");

    let full = load_journal(&fixture.run_id, final_records, final_objects)
        .expect("independent full journal")
        .verify(certified_history_spec())
        .expect("independent full certified fold");
    assert_fact_projection_equivalent(&incrementally_advanced, &full, &fixture);
    assert_eq!(incrementally_advanced.current_run_sequence(), Some(5));
    assert_eq!(
        incrementally_advanced.current_run_sequence(),
        full.current_run_sequence()
    );
}

fn fact_descriptor_object_key(
    fixture: &HistoryFixture,
) -> ((ArtifactId, ContentDigest), (Vec<u8>, ArtifactEvidenceRef)) {
    fixture
        .objects
        .iter()
        .find(|(_, (_, evidence))| evidence.artifact_role == ArtifactRole::FactDescriptor)
        .map(|(key, value)| (key.clone(), value.clone()))
        .expect("fact descriptor object")
}

fn load_corrupt_fact_successor(
    mut mutate: impl FnMut(
        &mut ArtifactByteAuthorityMap,
        (ArtifactId, ContentDigest),
        (Vec<u8>, ArtifactEvidenceRef),
    ),
) -> StoreError {
    let fixture = history_fixture(&[HistoryFact::new("account-a", 41)]);
    let (descriptor_key, descriptor_object) = fact_descriptor_object_key(&fixture);
    let initial = load_journal(
        &fixture.run_id,
        fixture.records[..1].to_vec(),
        fixture.objects.clone(),
    )
    .expect("valid prefix journal");
    let _view = initial
        .verify(fixture.certified)
        .expect("valid prefix certified view");
    let mut successor_objects = fixture.objects;
    mutate(&mut successor_objects, descriptor_key, descriptor_object);
    load_journal(&fixture.run_id, fixture.records, successor_objects)
        .expect_err("corrupt prefix-retained descriptor cannot mint successor authority")
}

#[test]
fn fact_successor_rejects_missing_wrong_key_and_tampered_prefix_descriptor_objects() {
    let missing = load_corrupt_fact_successor(|objects, key, _| {
        objects.remove(&key);
    });
    assert!(matches!(missing, StoreError::MissingArtifact { .. }));

    let wrong_key = load_corrupt_fact_successor(|objects, key, object| {
        objects.remove(&key);
        objects.insert((key.0, fixed_content_digest_for_test(0x69)), object);
    });
    assert!(
        matches!(
            wrong_key,
            StoreError::MissingArtifact { .. } | StoreError::ArtifactEvidenceMismatch { .. }
        ),
        "exact lookup must not scan or fall back: {wrong_key:?}"
    );

    let tampered_bytes = load_corrupt_fact_successor(|objects, key, _| {
        objects
            .get_mut(&key)
            .expect("descriptor object")
            .0
            .push(b'!');
    });
    assert!(matches!(
        tampered_bytes,
        StoreError::ArtifactEvidenceMismatch { field: "bytes", .. }
    ));

    let tampered_evidence = load_corrupt_fact_successor(|objects, key, _| {
        objects.get_mut(&key).expect("descriptor object").1.byte_len += 1;
    });
    assert!(matches!(
        tampered_evidence,
        StoreError::ArtifactEvidenceMismatch { .. }
    ));
}
