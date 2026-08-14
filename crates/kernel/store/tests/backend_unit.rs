use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use super::*;
use mfm_canonical::raw_content_digest;
use mfm_ids::{DigestAlgorithm, DigestBytes, SchemaId, StableId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
struct BackendValue {
    value: u64,
}

#[derive(Debug, Serialize, Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
struct OtherBackendValue {
    value: u64,
}

struct BackendRead;

impl mfm_capabilities::AccessCapabilityContract for BackendRead {
    type Mode = mfm_capabilities::ReadMode;
    type Intent = BackendValue;
    type Evidence = BackendValue;
    type Facts = mfm_capabilities::NoPriorFacts;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.backend-read")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }

    fn total_attempt_bound() -> NonZeroU16 {
        NonZeroU16::new(2).expect("nonzero")
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (intent.value == evidence.value)
            .then_some(())
            .ok_or(mfm_capabilities::CapabilityError::EvidenceBinding)
    }
}

#[derive(Debug, Serialize, Deserialize, mfm_program_derive::MfmConfig)]
#[serde(deny_unknown_fields)]
struct BackendConfig {
    value: u64,
}

static CONFIG_DECODES: AtomicUsize = AtomicUsize::new(0);
static CONFIG_VALIDATIONS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Serialize, mfm_program_derive::MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(validate = "validate_counted_config")]
struct CountedConfig {
    value: u64,
}

fn validate_counted_config(_value: &CountedConfig) -> std::result::Result<(), &'static str> {
    CONFIG_VALIDATIONS.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

impl<'de> Deserialize<'de> for CountedConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            value: u64,
        }

        let wire = Wire::deserialize(deserializer)?;
        CONFIG_DECODES.fetch_add(1, Ordering::SeqCst);
        Ok(Self { value: wire.value })
    }
}

#[derive(Debug, Serialize, Deserialize, mfm_program_derive::MfmConfig)]
#[serde(deny_unknown_fields)]
struct StringConfig {
    value: String,
}

fn backend_catalog_builder() -> mfm_program::ProgramCatalogBuilder {
    let mut builder = ProgramCatalog::builder();
    builder
        .register_value::<BackendValue>()
        .expect("backend value");
    builder
        .register_capability::<BackendRead>()
        .expect("backend read");
    builder
}

fn identity() -> StructuredStoreIdentity {
    StructuredStoreIdentity::new(
        StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef").expect("scope"),
        StoreEpoch::new(1),
        TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef").expect("tenant"),
    )
}

#[test]
fn store_rejects_catalog_qualified_output_for_another_contract() {
    let mut builder = backend_catalog_builder();
    let other_contract = builder
        .register_value::<OtherBackendValue>()
        .expect("other value");
    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let occurrence = mfm_program::SequentialControlAddress::new(0, Vec::new()).expect("occurrence");
    let document = mfm_program::ProgramDocument::new(
        StableId::new("mfm.test.typed-output").expect("entry"),
        contract.clone(),
        contract.clone(),
        vec![mfm_program::Declaration::State(Box::new(
            mfm_program::StateDeclaration::new(
                occurrence.clone(),
                fact_source(1),
                contract.clone(),
                contract,
                None,
                mfm_program::ExecutionMode::Pure,
                true,
            )
            .expect("state"),
        ))],
    )
    .expect("document");
    let (catalog, _) = builder.finish(document.clone()).expect("catalog");
    let output = catalog
        .qualify(other_contract, OtherBackendValue { value: 1 })
        .expect("qualified")
        .erase();
    assert!(matches!(
        proposed_outcome(
            &catalog,
            &document,
            &occurrence,
            ProposedStateOutcome::Success {
                output,
                facts: FactProposalSet::empty(),
            },
        ),
        Err(StoreError::InvalidRecord)
    ));
}

#[tokio::test]
async fn store_rejects_correct_schema_with_invalid_retained_output_bytes() {
    let identity = identity();
    let base = admission_frame(&identity);
    let RunRecord::RunAdmitted(base_admission) = base.record() else {
        panic!("expected admission")
    };
    let contract = base_admission.admitted_context().contract_ref().clone();
    let occurrence = mfm_program::SequentialControlAddress::new(0, Vec::new()).expect("occurrence");
    let document = mfm_program::ProgramDocument::new(
        base_admission.entry_point_id().clone(),
        contract.clone(),
        contract.clone(),
        vec![mfm_program::Declaration::State(Box::new(
            mfm_program::StateDeclaration::new(
                occurrence.clone(),
                fact_source(1),
                contract.clone(),
                contract.clone(),
                None,
                mfm_program::ExecutionMode::Pure,
                true,
            )
            .expect("state"),
        ))],
    )
    .expect("document");
    let admission = mfm_journal::RunAdmitted::new(
        identity.scope().clone(),
        identity.epoch(),
        base.run_id().clone(),
        identity.tenant().clone(),
        base_admission.entry_point_id().clone(),
        document.program_ref().expect("program ref"),
        base_admission.admitted_context().clone(),
        base_admission.configuration().clone(),
        Vec::new(),
    )
    .expect("admission");
    let admission = mfm_journal::RunFrame::new(
        base.run_id().clone(),
        identity.scope().clone(),
        identity.epoch(),
        1,
        AppendRequestId::new("invalid-output-admission-0123456789").expect("request"),
        RunRecord::RunAdmitted(admission),
        vec![base.objects()[0].clone(), program_object(&document)],
    )
    .expect("admission frame");
    let (catalog, _) = backend_catalog_builder()
        .finish(document.clone())
        .expect("catalog");
    let opened =
        StructuredStore::open_memory(identity.clone(), catalog, StoreWorkLimits::default())
            .expect("opened store");
    let configuration = configuration_head(&opened).await;
    opened
        .test_append_admission(admission.clone(), &configuration)
        .await
        .expect("append admission");

    let invalid_bytes = r#"{"value":"not-a-number"}"#;
    let invalid_content = ContentRef::new(
        contract.schema_id().clone(),
        raw_content_digest(invalid_bytes.as_bytes()),
    )
    .expect("invalid content");
    let invalid_output = ValueRef::new(contract, invalid_content.clone());
    let invalid_object = ImmutableObject::new(
        StableId::new("mfm.value").expect("object type"),
        invalid_content,
        invalid_bytes.to_owned(),
    )
    .expect("invalid object");
    let conclusion = mfm_journal::RunFrame::new(
        base.run_id().clone(),
        identity.scope().clone(),
        identity.epoch(),
        2,
        AppendRequestId::new("invalid-output-conclusion-0123456789").expect("request"),
        RunRecord::StateConcluded(mfm_journal::StateConcluded::Pure {
            occurrence,
            outcome: StateOutcome::Success(invalid_output),
            fact_proposals: None,
            fact_publication: None,
        }),
        vec![invalid_object],
    )
    .expect("conclusion frame");
    opened
        .history_port()
        .append_frame(conclusion)
        .await
        .expect("append structurally valid conclusion");
    let retained = opened.load(base.run_id()).await.expect("retained history");
    assert!(matches!(
        opened.test_select_qualified(&retained),
        Err(StoreError::InvalidHistory)
    ));
}

fn admission_frame(identity: &StructuredStoreIdentity) -> mfm_journal::single_trust::RunFrame {
    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let entry = mfm_ids::StableId::new("mfm.test.backend-limit-entry").expect("entry");
    let document = mfm_program::ProgramDocument::new(
        entry.clone(),
        contract.clone(),
        contract.clone(),
        Vec::new(),
    )
    .expect("document");
    let value_bytes = br#"{"value":1}"#;
    let value = ContentRef::new(
        contract.schema_id().clone(),
        raw_content_digest(value_bytes),
    )
    .expect("value");
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .expect("run");
    let configuration_bytes = br#"{"value":1}"#;
    let configuration_ref = ContentRef::new(
        BackendConfig::schema_id().expect("configuration schema"),
        raw_content_digest(configuration_bytes),
    )
    .expect("configuration");
    let admitted = mfm_journal::single_trust::RunAdmitted::new(
        identity.scope().clone(),
        identity.epoch(),
        run_id.clone(),
        identity.tenant().clone(),
        entry,
        document.program_ref().expect("program ref"),
        mfm_journal::single_trust::ValueRef::new(contract.clone(), value.clone()),
        ConfigurationHeadProjection::new(1, configuration_ref).expect("configuration head"),
        Vec::new(),
    )
    .expect("admission");
    mfm_journal::single_trust::RunFrame::new(
        run_id,
        identity.scope().clone(),
        identity.epoch(),
        1,
        AppendRequestId::new("backend-limit-admission-0123456789").expect("request"),
        mfm_journal::single_trust::RunRecord::RunAdmitted(admitted),
        vec![
            mfm_journal::single_trust::ImmutableObject::new(
                mfm_ids::StableId::new("mfm.value").expect("object"),
                value,
                String::from_utf8(value_bytes.to_vec()).expect("value bytes"),
            )
            .expect("object"),
            program_object(&document),
        ],
    )
    .expect("frame")
}

fn program_object(document: &mfm_program::ProgramDocument) -> ImmutableObject {
    ImmutableObject::new(
        StableId::new("mfm.program").expect("program object type"),
        document.program_ref().expect("program ref"),
        document
            .canonical_bytes()
            .expect("program bytes")
            .as_str()
            .to_owned(),
    )
    .expect("program object")
}

fn fact_source(seed: u8) -> ContentRef {
    let schema = SchemaId::new(
        "mfm.test.fact-source",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([seed; 32]),
    )
    .expect("source schema");
    ContentRef::new(schema, raw_content_digest(&[seed])).expect("source")
}

fn sourced_admission_frame(
    identity: &StructuredStoreIdentity,
    source: ContentRef,
) -> mfm_journal::single_trust::RunFrame {
    let base = admission_frame(identity);
    let RunRecord::RunAdmitted(admitted) = base.record() else {
        panic!("expected admission")
    };
    let admission = mfm_journal::single_trust::RunAdmitted::new(
        identity.scope().clone(),
        identity.epoch(),
        base.run_id().clone(),
        identity.tenant().clone(),
        admitted.entry_point_id().clone(),
        admitted.program_ref().clone(),
        admitted.admitted_context().clone(),
        admitted.configuration().clone(),
        vec![source],
    )
    .expect("sourced admission");
    mfm_journal::single_trust::RunFrame::new(
        base.run_id().clone(),
        identity.scope().clone(),
        identity.epoch(),
        1,
        AppendRequestId::new("backend-fact-admission-0123456789").expect("request"),
        RunRecord::RunAdmitted(admission),
        base.objects().to_vec(),
    )
    .expect("sourced frame")
}

async fn configuration_head(opened: &OpenedStructuredStore) -> ResolvedConfigurationHead {
    let configuration = opened.test_configuration();
    let owner = configuration
        .initial_write_session::<BackendConfig>()
        .prepare_local(
            AppendRequestId::new("backend-test-configuration-000001").expect("request"),
            ValidatedConfig::new(BackendConfig { value: 1 }).expect("config"),
        )
        .expect("owner");
    match configuration
        .commit(owner)
        .await
        .expect("configuration commit")
    {
        ConfigurationCommitOutcome::NewlyCommitted(resolved)
        | ConfigurationCommitOutcome::Found(resolved) => resolved.into_head(),
        other => panic!("unexpected configuration outcome: {other:?}"),
    }
}

#[test]
fn outer_capacity_accepts_exact_ceiling_and_rejects_each_plus_one() {
    let exact = StoreWorkLimits::default();
    assert_eq!(exact.validate(), Ok(()));

    let cases = [
        StoreWorkLimits::new(
            mfm_journal::single_trust::MAX_FRAME_BYTES + 1,
            mfm_journal::single_trust::MAX_RUN_FRAMES,
            mfm_journal::single_trust::MAX_RUN_OBJECTS,
            mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
        ),
        StoreWorkLimits::new(
            mfm_journal::single_trust::MAX_FRAME_BYTES,
            mfm_journal::single_trust::MAX_RUN_FRAMES + 1,
            mfm_journal::single_trust::MAX_RUN_OBJECTS,
            mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
        ),
        StoreWorkLimits::new(
            mfm_journal::single_trust::MAX_FRAME_BYTES,
            mfm_journal::single_trust::MAX_RUN_FRAMES,
            mfm_journal::single_trust::MAX_RUN_OBJECTS + 1,
            mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
        ),
        StoreWorkLimits::new(
            mfm_journal::single_trust::MAX_FRAME_BYTES,
            mfm_journal::single_trust::MAX_RUN_FRAMES,
            mfm_journal::single_trust::MAX_RUN_OBJECTS,
            mfm_journal::single_trust::MAX_RUN_FRAME_BYTES + 1,
        ),
    ];
    assert!(cases.iter().all(|limits| limits.validate().is_err()));
    eprintln!(
        "capacity-envelope store frame_bytes={} frames={} objects={} run_frame_bytes={}",
        exact.max_frame_bytes(),
        exact.max_run_frames(),
        exact.max_run_objects(),
        exact.max_run_frame_bytes(),
    );
}

fn proposal(seed: u8) -> (ValueRef, mfm_journal::single_trust::ImmutableObject) {
    let source = fact_source(seed);
    let subject = fact_source(seed.saturating_add(1));
    let canonical_value = format!(r#"{{"value":{seed}}}"#);
    let value_ref =
        mfm_facts::content_ref_for_value(canonical_value.as_bytes()).expect("fact value ref");
    let fact = mfm_facts::FactValue::new(
        source,
        subject,
        mfm_ids::StableId::new(format!("mfm.test.fact-{seed}")).expect("fact id"),
        value_ref,
        canonical_value,
    )
    .expect("fact");
    let proposals = mfm_facts::FactProposalSet::new(vec![fact]).expect("proposals");
    let canonical =
        mfm_journal::single_trust::canonical_json(&proposals).expect("proposal canonical");
    let proposal_ref = ContentRef::new(
        mfm_facts::FactProposalSet::schema_id().expect("proposal schema"),
        raw_content_digest(canonical.as_bytes()),
    )
    .expect("proposal ref");
    let object = mfm_journal::single_trust::ImmutableObject::new(
        mfm_ids::StableId::new("mfm.value").expect("object type"),
        proposal_ref.clone(),
        canonical.as_str().to_owned(),
    )
    .expect("proposal object");
    (ValueRef::new(proposal_ref.clone(), proposal_ref), object)
}

struct InjectingFactBackend {
    inner: Arc<MemoryStructuredBackend>,
    injected: AtomicU8,
}

impl InjectingFactBackend {
    fn new(inner: Arc<MemoryStructuredBackend>) -> Self {
        Self {
            inner,
            injected: AtomicU8::new(0),
        }
    }
}

impl StructuredStoreBackend for InjectingFactBackend {
    fn identity(&self) -> StructuredStoreIdentity {
        self.inner.identity()
    }

    fn load_complete_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        limit: RawHistoryLoadLimit,
    ) -> BackendFuture<'a, Option<RawRunPrefix>> {
        self.inner.load_complete_prefix(run_id, limit)
    }

    fn compare_and_append<'a>(
        &'a self,
        command: &'a BackendAppendCommand<'a>,
    ) -> BackendFuture<'a, BackendAppendOutcome> {
        let inner = Arc::clone(&self.inner);
        let injection_index = command
            .fact_publication()
            .map_or(2, |_| self.injected.fetch_add(1, Ordering::SeqCst));
        let inject = injection_index < 2;
        Box::pin(async move {
            if inject {
                let identity = inner.identity();
                let run_id = RunId::parse(match injection_index {
                        0 => {
                            "run:sha256-jcs-v1:5123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                        }
                        _ => {
                            "run:sha256-jcs-v1:5223456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                        }
                    })
                    .map_err(|_| BackendError::Storage)?;
                let publication_sequence = inner.load_facts().await?.head_sequence() + 1;
                let bytes = br#"{}"#;
                let frame_digest = raw_content_digest(bytes);
                let head_digest = raw_content_digest(b"injected-head");
                let request = AppendRequestId::new("injected-fact-0123456789abcdef")
                    .map_err(|_| BackendError::Storage)?;
                let schema = SchemaId::new(
                    "mfm.test.injected-proposals",
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([9; 32]),
                )
                .map_err(|_| BackendError::Storage)?;
                let proposal_ref = ContentRef::new(schema, raw_content_digest(b"injected"))
                    .map_err(|_| BackendError::Storage)?;
                let publication =
                    RawFactPublication::new(publication_sequence, run_id.clone(), 1, proposal_ref)?;
                let injected = BackendAppendCommand::new(
                    &identity,
                    &run_id,
                    1,
                    &request,
                    bytes,
                    &frame_digest,
                    &head_digest,
                    None,
                    true,
                    Some(&publication),
                );
                if !matches!(
                    inner.compare_and_append(&injected).await?,
                    BackendAppendOutcome::NewlyCommitted
                ) {
                    return Err(BackendError::Conflict);
                }
            }
            inner.compare_and_append(command).await
        })
    }

    fn load_configuration<'a>(&'a self) -> BackendFuture<'a, Vec<RawConfigurationRevision>> {
        self.inner.load_configuration()
    }

    fn compare_and_append_configuration<'a>(
        &'a self,
        command: &'a ConfigurationAppendCommand<'a>,
    ) -> BackendFuture<'a, BackendConfigurationOutcome> {
        self.inner.compare_and_append_configuration(command)
    }

    fn load_facts<'a>(&'a self) -> BackendFuture<'a, RawFactSnapshot> {
        self.inner.load_facts()
    }

    fn audit_run_ids<'a>(&'a self) -> BackendFuture<'a, Vec<RunId>> {
        self.inner.audit_run_ids()
    }
}

struct UnknownHistoryOnceBackend {
    inner: Arc<MemoryStructuredBackend>,
    unknown: AtomicU8,
}

impl UnknownHistoryOnceBackend {
    fn new(inner: Arc<MemoryStructuredBackend>) -> Self {
        Self {
            inner,
            unknown: AtomicU8::new(1),
        }
    }
}

impl StructuredStoreBackend for UnknownHistoryOnceBackend {
    fn identity(&self) -> StructuredStoreIdentity {
        self.inner.identity()
    }

    fn load_complete_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        limit: RawHistoryLoadLimit,
    ) -> BackendFuture<'a, Option<RawRunPrefix>> {
        self.inner.load_complete_prefix(run_id, limit)
    }

    fn compare_and_append<'a>(
        &'a self,
        command: &'a BackendAppendCommand<'a>,
    ) -> BackendFuture<'a, BackendAppendOutcome> {
        Box::pin(async move {
            let outcome = self.inner.compare_and_append(command).await?;
            if matches!(outcome, BackendAppendOutcome::NewlyCommitted)
                && self.unknown.swap(0, Ordering::SeqCst) == 1
            {
                return Ok(BackendAppendOutcome::AcknowledgementUnknown);
            }
            Ok(outcome)
        })
    }

    fn load_configuration<'a>(&'a self) -> BackendFuture<'a, Vec<RawConfigurationRevision>> {
        self.inner.load_configuration()
    }

    fn compare_and_append_configuration<'a>(
        &'a self,
        command: &'a ConfigurationAppendCommand<'a>,
    ) -> BackendFuture<'a, BackendConfigurationOutcome> {
        self.inner.compare_and_append_configuration(command)
    }

    fn load_facts<'a>(&'a self) -> BackendFuture<'a, RawFactSnapshot> {
        self.inner.load_facts()
    }

    fn audit_run_ids<'a>(&'a self) -> BackendFuture<'a, Vec<RunId>> {
        self.inner.audit_run_ids()
    }
}

#[tokio::test]
async fn opened_limits_reject_frame_before_backend_ingress() {
    let identity = identity();
    let frame = admission_frame(&identity);
    let frame_bytes = frame
        .canonical_bytes()
        .expect("canonical frame")
        .as_bytes()
        .len();
    let root_contract = match frame.record() {
        mfm_journal::single_trust::RunRecord::RunAdmitted(admitted) => {
            admitted.admitted_context().contract_ref().clone()
        }
        _ => panic!("expected admission"),
    };
    let document = mfm_program::single_trust::ProgramDocument::new(
        mfm_ids::StableId::new("mfm.test.backend-limit-entry").expect("entry"),
        root_contract.clone(),
        root_contract,
        Vec::new(),
    )
    .expect("document");
    let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
    let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
    let opened = StructuredStore::open(
        backend.clone(),
        identity.clone(),
        catalog,
        StoreWorkLimits::new(
            frame_bytes.saturating_sub(1),
            mfm_journal::single_trust::MAX_RUN_FRAMES,
            mfm_journal::single_trust::MAX_RUN_OBJECTS,
            mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
        ),
    )
    .await
    .expect("opened store");
    let configuration = configuration_head(&opened).await;
    assert_eq!(
        opened.test_append_admission(frame, &configuration).await,
        Err(StoreError::Capacity)
    );
    assert!(backend
            .load_complete_prefix(
                &RunId::parse(
                    "run:sha256-jcs-v1:4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                )
                .expect("run"),
                RawHistoryLoadLimit::new(
                    mfm_journal::single_trust::MAX_RUN_FRAMES,
                    mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
                ),
            )
            .await
            .expect("backend load")
            .is_none());
}

#[tokio::test]
async fn admission_port_rejects_non_genesis_frames_before_backend_ingress() {
    let identity = identity();
    let admission = admission_frame(&identity);
    let (contract_ref, value_ref, value_json) = match admission.record() {
        mfm_journal::single_trust::RunRecord::RunAdmitted(admitted) => {
            let value = admitted.admitted_context().value_ref().clone();
            (
                admitted.admitted_context().contract_ref().clone(),
                value,
                r#"{"value":1}"#,
            )
        }
        _ => panic!("expected admission"),
    };
    let non_genesis = mfm_journal::single_trust::RunFrame::new(
        admission.run_id().clone(),
        identity.scope().clone(),
        identity.epoch(),
        2,
        AppendRequestId::new("backend-admission-port-0123456789").expect("request"),
        mfm_journal::single_trust::RunRecord::StateConcluded(
            mfm_journal::single_trust::StateConcluded::Pure {
                occurrence: mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
                    .expect("occurrence"),
                outcome: mfm_journal::single_trust::StateOutcome::Success(
                    mfm_journal::single_trust::ValueRef::new(
                        contract_ref.clone(),
                        value_ref.clone(),
                    ),
                ),
                fact_proposals: None,
                fact_publication: None,
            },
        ),
        vec![mfm_journal::single_trust::ImmutableObject::new(
            mfm_ids::StableId::new("mfm.value").expect("object"),
            value_ref,
            value_json.to_owned(),
        )
        .expect("object")],
    )
    .expect("non-genesis frame");
    let (catalog, _) = backend_catalog_builder()
        .finish(
            mfm_program::single_trust::ProgramDocument::new(
                mfm_ids::StableId::new("mfm.test.backend-limit-entry").expect("entry"),
                contract_ref.clone(),
                contract_ref,
                Vec::new(),
            )
            .expect("document"),
        )
        .expect("catalog");
    let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
    let opened = StructuredStore::open(
        backend.clone(),
        identity,
        catalog,
        StoreWorkLimits::default(),
    )
    .await
    .expect("opened store");
    let configuration = configuration_head(&opened).await;
    assert_eq!(
        opened
            .test_append_admission(non_genesis, &configuration)
            .await,
        Err(StoreError::InvalidRecord)
    );
    assert!(backend
        .load_complete_prefix(
            admission.run_id(),
            RawHistoryLoadLimit::new(
                mfm_journal::single_trust::MAX_RUN_FRAMES,
                mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
            ),
        )
        .await
        .expect("backend load")
        .is_none());
}

#[tokio::test]
async fn store_built_admission_retains_owner_across_unknown_acknowledgement() {
    let identity = identity();
    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let document = mfm_program::ProgramDocument::new(
        StableId::new("mfm.test.store-built-admission-entry").expect("entry"),
        contract.clone(),
        contract.clone(),
        Vec::new(),
    )
    .expect("document");
    let (catalog, program) = backend_catalog_builder().finish(document).expect("catalog");
    let value = catalog
        .qualify(contract, BackendValue { value: 7 })
        .expect("qualified value");
    let backend = Arc::new(UnknownHistoryOnceBackend::new(Arc::new(
        MemoryStructuredBackend::new(identity.clone()),
    )));
    let opened = StructuredStore::open(
        backend.clone(),
        identity,
        catalog,
        StoreWorkLimits::default(),
    )
    .await
    .expect("opened store");
    let configuration = configuration_head(&opened).await;
    let (history, reader, _, _) = opened.split().into_parts();
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:7123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .expect("run id");

    let owner = match history
        .admit(run_id.clone(), &program, &value, configuration, Vec::new())
        .await
    {
        AdmissionOutcome::AcknowledgementUnknown(owner) => owner,
        other => panic!("unexpected admission outcome: {other:?}"),
    };
    let selected = match history.resolve_admission(owner).await {
        AdmissionOutcome::Selected(selected, AppendDisposition::Found { sequence: 1 }) => selected,
        other => panic!("unexpected admission resolution: {other:?}"),
    };
    assert_eq!(selected.head_sequence(), 1);
    assert_eq!(selected.program_ref(), program.program_ref().content_ref());
    let retained = reader.load(&run_id).await.expect("retained admission");
    assert_eq!(retained.frames()[0].expected_sequence(), 1);
    let program_objects: Vec<_> = retained.frames()[0]
        .objects()
        .iter()
        .filter(|object| object.object_type().as_str() == "mfm.program")
        .collect();
    assert_eq!(program_objects.len(), 1);
    assert_eq!(
        program_objects[0].content_ref(),
        program.program_ref().content_ref()
    );
    assert_eq!(
        program_objects[0].canonical_json(),
        program
            .document()
            .canonical_bytes()
            .expect("program bytes")
            .as_str()
    );
    assert!(retained.frames()[0]
        .append_request_id()
        .as_str()
        .starts_with("admission-"));
}

#[tokio::test]
async fn retained_program_rejects_a_catalog_without_its_registered_contracts() {
    let identity = identity();
    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let document = mfm_program::ProgramDocument::new(
        StableId::new("mfm.test.foreign-catalog-entry").expect("entry"),
        contract.clone(),
        contract.clone(),
        Vec::new(),
    )
    .expect("document");
    let (catalog, program) = backend_catalog_builder().finish(document).expect("catalog");
    let value = catalog
        .qualify(contract, BackendValue { value: 8 })
        .expect("qualified value");
    let opened = StructuredStore::open_memory(identity, catalog, StoreWorkLimits::default())
        .expect("opened store");
    let configuration = configuration_head(&opened).await;
    let (history, reader, _, _) = opened.split().into_parts();
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:8123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .expect("run id");
    assert!(matches!(
        history
            .admit(run_id.clone(), &program, &value, configuration, Vec::new())
            .await,
        AdmissionOutcome::Selected(_, _)
    ));
    let retained = reader.load(&run_id).await.expect("retained run");

    let mut foreign_builder = ProgramCatalog::builder();
    let foreign_contract = foreign_builder
        .register_value::<OtherBackendValue>()
        .expect("foreign registration");
    let foreign_document = mfm_program::ProgramDocument::new(
        StableId::new("mfm.test.foreign-catalog-entry").expect("entry"),
        foreign_contract.clone(),
        foreign_contract,
        Vec::new(),
    )
    .expect("foreign document");
    let (foreign_catalog, _) = foreign_builder
        .finish(foreign_document)
        .expect("foreign catalog");
    assert!(matches!(
        retained_program(&retained, &foreign_catalog),
        Err(StoreError::InvalidHistory)
    ));
}

#[tokio::test]
async fn qualified_history_and_reduction_cannot_cross_store_openings() {
    let identity = identity();
    let frame = admission_frame(&identity);
    let (contract, value_ref) = match frame.record() {
        mfm_journal::single_trust::RunRecord::RunAdmitted(admitted) => (
            admitted.admitted_context().contract_ref().clone(),
            admitted.admitted_context().value_ref().clone(),
        ),
        _ => panic!("expected admission"),
    };
    let (catalog, _) = backend_catalog_builder()
        .finish(
            mfm_program::single_trust::ProgramDocument::new(
                mfm_ids::StableId::new("mfm.test.backend-limit-entry").expect("entry"),
                contract.clone(),
                contract.clone(),
                Vec::new(),
            )
            .expect("document"),
        )
        .expect("catalog");
    let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
    let first = StructuredStore::open(
        backend.clone(),
        identity.clone(),
        catalog.clone(),
        StoreWorkLimits::default(),
    )
    .await
    .expect("first opening");
    let second = StructuredStore::open(
        backend,
        identity,
        catalog.clone(),
        StoreWorkLimits::default(),
    )
    .await
    .expect("second opening");
    let first_configuration = configuration_head(&first).await;
    assert!(!first.test_same_open(&second));
    first
        .test_append_admission(frame.clone(), &first_configuration)
        .await
        .expect("admission");
    let first_run = first.load(frame.run_id()).await.expect("first run");
    let second_run = second.load(frame.run_id()).await.expect("second run");
    assert_eq!(first_run.frames(), second_run.frames());
    let foreign_candidate = mfm_journal::single_trust::RunFrame::new(
        frame.run_id().clone(),
        frame.store_scope_id().clone(),
        frame.store_epoch(),
        2,
        AppendRequestId::new("backend-store-brand-0123456789").expect("request"),
        mfm_journal::single_trust::RunRecord::StateConcluded(
            mfm_journal::single_trust::StateConcluded::Pure {
                occurrence: mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
                    .expect("occurrence"),
                outcome: mfm_journal::single_trust::StateOutcome::Success(
                    mfm_journal::single_trust::ValueRef::new(contract.clone(), value_ref.clone()),
                ),
                fact_proposals: None,
                fact_publication: None,
            },
        ),
        vec![mfm_journal::single_trust::ImmutableObject::new(
            mfm_ids::StableId::new("mfm.value").expect("object"),
            value_ref,
            r#"{"value":1}"#.to_owned(),
        )
        .expect("object")],
    )
    .expect("candidate");
    assert!(matches!(
        second.test_qualify_appended(&first_run, foreign_candidate),
        Err(StoreError::Identity)
    ));
}

#[tokio::test]
async fn prior_fact_scan_reads_state_proposal_publications() {
    let identity = identity();
    let source = fact_source(40);
    let admission = sourced_admission_frame(&identity, source.clone());
    let (entry, contract) = match admission.record() {
        RunRecord::RunAdmitted(admitted) => (
            admitted.entry_point_id().clone(),
            admitted.admitted_context().contract_ref().clone(),
        ),
        _ => panic!("expected admission"),
    };
    let document = mfm_program::single_trust::ProgramDocument::new(
        entry,
        contract.clone(),
        contract,
        Vec::new(),
    )
    .expect("document");
    let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
    let opened =
        StructuredStore::open_memory(identity.clone(), catalog, StoreWorkLimits::default())
            .expect("opened store");
    let configuration = configuration_head(&opened).await;
    opened
        .test_append_admission(admission.clone(), &configuration)
        .await
        .expect("admission");

    let subject = fact_source(41);
    let value_ref = mfm_facts::content_ref_for_value(br#"{"value":1}"#).expect("fact value");
    let fact = mfm_facts::FactValue::new(
        source.clone(),
        subject.clone(),
        mfm_ids::StableId::new("mfm.test.fact").expect("fact id"),
        value_ref,
        r#"{"value":1}"#.to_owned(),
    )
    .expect("fact");
    let proposals = mfm_facts::FactProposalSet::new(vec![fact]).expect("proposals");
    let canonical =
        mfm_journal::single_trust::canonical_json(&proposals).expect("proposal canonical");
    let proposal_content_ref = ContentRef::new(
        mfm_facts::FactProposalSet::schema_id().expect("proposal schema"),
        raw_content_digest(canonical.as_bytes()),
    )
    .expect("proposal ref");
    let proposal_value = ValueRef::new(proposal_content_ref.clone(), proposal_content_ref.clone());
    let proposal_object = mfm_journal::single_trust::ImmutableObject::new(
        mfm_ids::StableId::new("mfm.value").expect("object type"),
        proposal_content_ref.clone(),
        canonical.as_str().to_owned(),
    )
    .expect("proposal object");
    let RunRecord::RunAdmitted(admitted) = admission.record() else {
        panic!("expected admission")
    };
    let outcome = admitted.admitted_context().clone();
    let conclusion = mfm_journal::single_trust::StateConcluded::Pure {
        occurrence: mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
            .expect("occurrence"),
        outcome: mfm_journal::single_trust::StateOutcome::Success(outcome.clone()),
        fact_proposals: Some(proposal_value.clone()),
        fact_publication: Some(
            mfm_journal::single_trust::FactPublication::new(1, proposal_value)
                .expect("publication"),
        ),
    };
    let conclusion_frame = mfm_journal::single_trust::RunFrame::new(
        admission.run_id().clone(),
        identity.scope().clone(),
        identity.epoch(),
        2,
        AppendRequestId::new("backend-fact-conclusion-0123456789").expect("request"),
        RunRecord::StateConcluded(conclusion),
        vec![admission.objects()[0].clone(), proposal_object],
    )
    .expect("conclusion frame");
    assert_eq!(
        opened
            .history_port()
            .append_frame(conclusion_frame)
            .await
            .expect("publication append"),
        AppendDisposition::NewlyCommitted { sequence: 2 }
    );

    let current = opened.load(admission.run_id()).await.expect("current");
    let request = mfm_facts::FactSelectionRequest::new(
        mfm_ids::StableId::new("mfm.test.request").expect("request id"),
        vec![source],
        subject,
    )
    .expect("selection request");
    let (selection_ref, selection_object) = opened
        .select_prior_facts(&current, &request)
        .await
        .expect("select prior facts");
    assert_eq!(
        selection_ref.contract_ref().schema_id(),
        &mfm_facts::FactSelection::schema_id().expect("selection schema")
    );
    let selection: mfm_facts::FactSelection =
        serde_json::from_str(selection_object.canonical_json()).expect("selection");
    selection
        .validate_for(&request)
        .expect("selection validation");
    assert_eq!(selection.facts.len(), 1);
    assert_eq!(selection.frontier.through_sequence, 1);
}

#[tokio::test]
async fn conclusion_rebinds_only_its_fact_coordinate_after_frontier_race() {
    let identity = identity();
    let base = admission_frame(&identity);
    let (entry, contract, context, configuration) = match base.record() {
        RunRecord::RunAdmitted(admitted) => (
            admitted.entry_point_id().clone(),
            admitted.admitted_context().contract_ref().clone(),
            admitted.admitted_context().clone(),
            admitted.configuration().clone(),
        ),
        _ => panic!("expected admission"),
    };
    let state = mfm_program::single_trust::StateDeclaration::new(
        mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
            .expect("occurrence"),
        fact_source(80),
        contract.clone(),
        contract.clone(),
        None,
        mfm_program::single_trust::ExecutionMode::Pure,
        true,
    )
    .expect("state");
    let document = mfm_program::single_trust::ProgramDocument::new(
        entry,
        contract.clone(),
        contract,
        vec![mfm_program::Declaration::State(Box::new(state))],
    )
    .expect("document");
    let program_ref = document.program_ref().expect("program ref");
    let admission = mfm_journal::single_trust::RunFrame::new(
        base.run_id().clone(),
        identity.scope().clone(),
        identity.epoch(),
        1,
        AppendRequestId::new("backend-rebind-admission-0123456789").expect("request"),
        RunRecord::RunAdmitted(
            mfm_journal::single_trust::RunAdmitted::new(
                identity.scope().clone(),
                identity.epoch(),
                base.run_id().clone(),
                identity.tenant().clone(),
                match base.record() {
                    RunRecord::RunAdmitted(admitted) => admitted.entry_point_id().clone(),
                    _ => unreachable!(),
                },
                program_ref,
                context.clone(),
                configuration,
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![base.objects()[0].clone(), program_object(&document)],
    )
    .expect("admission frame");
    let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
    let injecting = Arc::new(InjectingFactBackend::new(Arc::clone(&backend)));
    let (catalog, _) = backend_catalog_builder()
        .finish(document.clone())
        .expect("catalog");
    let opened = StructuredStore::open(
        injecting,
        identity.clone(),
        catalog,
        StoreWorkLimits::default(),
    )
    .await
    .expect("opened store");
    let configuration = configuration_head(&opened).await;
    opened
        .test_append_admission(admission.clone(), &configuration)
        .await
        .expect("admission");
    let current = opened.load(admission.run_id()).await.expect("current");
    let selected = opened.test_select_qualified(&current).expect("selected");
    let (proposal_value, proposal_object) = proposal(90);
    let conclusion = mfm_journal::single_trust::StateConcluded::Pure {
        occurrence: mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
            .expect("occurrence"),
        outcome: mfm_journal::single_trust::StateOutcome::Success(context.clone()),
        fact_proposals: Some(proposal_value.clone()),
        fact_publication: None,
    };
    let owner = opened
        .test_prepare_conclusion_fixture(
            &current,
            &document,
            &selected,
            1,
            conclusion,
            vec![base.objects()[0].clone(), proposal_object],
            mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
        )
        .expect("conclusion owner");
    let original_append_request_id = owner.frame().append_request_id().clone();
    let owner = match opened.commit_conclusion(owner).await.expect("first commit") {
        ConclusionCommitOutcome::Rejected {
            owner,
            error: StoreError::FactFrontierChanged,
        } => {
            assert!(owner.frame().record().fact_publication().is_none());
            assert_ne!(
                owner.frame().append_request_id(),
                &original_append_request_id
            );
            owner
        }
        other => panic!("unexpected first commit: {other:?}"),
    };
    let first_rebound_append_request_id = owner.frame().append_request_id().clone();
    let owner = match opened
        .commit_conclusion(owner)
        .await
        .expect("second commit")
    {
        ConclusionCommitOutcome::Rejected {
            owner,
            error: StoreError::FactFrontierChanged,
        } => {
            assert!(owner.frame().record().fact_publication().is_none());
            assert_ne!(
                owner.frame().append_request_id(),
                &first_rebound_append_request_id
            );
            owner
        }
        other => panic!("unexpected second commit: {other:?}"),
    };
    let (disposition, frame) = match opened
        .commit_conclusion(owner)
        .await
        .expect("rebound commit")
    {
        ConclusionCommitOutcome::Disposition { disposition, frame } => (disposition, frame),
        other => panic!("unexpected rebound commit: {other:?}"),
    };
    assert_eq!(
        disposition,
        AppendDisposition::NewlyCommitted { sequence: 2 }
    );
    assert_eq!(
        frame
            .record()
            .fact_publication()
            .map(|publication| publication.publication_sequence()),
        Some(3)
    );
    assert_eq!(
        frame
            .record()
            .fact_publication()
            .map(|publication| publication.proposal_set_ref()),
        Some(proposal_value.clone()).as_ref()
    );
    let facts = opened.split().into_parts().3.facts().await.expect("facts");
    assert_eq!(facts.head_sequence(), 3);
    assert_eq!(facts.publications().len(), 3);
}

#[tokio::test]
async fn conclusion_head_race_classifies_same_and_conflicting_semantics() {
    let identity = identity();
    let base = admission_frame(&identity);
    let (entry, contract, context, configuration) = match base.record() {
        RunRecord::RunAdmitted(admitted) => (
            admitted.entry_point_id().clone(),
            admitted.admitted_context().contract_ref().clone(),
            admitted.admitted_context().clone(),
            admitted.configuration().clone(),
        ),
        _ => panic!("expected admission"),
    };
    let occurrence = mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
        .expect("occurrence");
    let state = mfm_program::single_trust::StateDeclaration::new(
        occurrence.clone(),
        fact_source(81),
        contract.clone(),
        contract.clone(),
        None,
        mfm_program::single_trust::ExecutionMode::Pure,
        true,
    )
    .expect("state");
    let document = mfm_program::single_trust::ProgramDocument::new(
        entry,
        contract.clone(),
        contract,
        vec![mfm_program::Declaration::State(Box::new(state))],
    )
    .expect("document");
    let program_ref = document.program_ref().expect("program ref");
    let admission = mfm_journal::single_trust::RunFrame::new(
        base.run_id().clone(),
        identity.scope().clone(),
        identity.epoch(),
        1,
        AppendRequestId::new("classification-admission-0123456789").expect("request"),
        RunRecord::RunAdmitted(
            mfm_journal::single_trust::RunAdmitted::new(
                identity.scope().clone(),
                identity.epoch(),
                base.run_id().clone(),
                identity.tenant().clone(),
                match base.record() {
                    RunRecord::RunAdmitted(admitted) => admitted.entry_point_id().clone(),
                    _ => unreachable!(),
                },
                program_ref,
                context.clone(),
                configuration,
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![base.objects()[0].clone(), program_object(&document)],
    )
    .expect("admission frame");
    let (catalog, _) = backend_catalog_builder()
        .finish(document.clone())
        .expect("catalog");
    let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
    let opened = StructuredStore::open(
        backend.clone(),
        identity.clone(),
        catalog.clone(),
        StoreWorkLimits::default(),
    )
    .await
    .expect("opened store");
    let second = StructuredStore::open(
        backend,
        identity,
        catalog.clone(),
        StoreWorkLimits::default(),
    )
    .await
    .expect("second opening");
    let configuration_head = configuration_head(&opened).await;
    opened
        .test_append_admission(admission.clone(), &configuration_head)
        .await
        .expect("admission append");
    let current = opened.load(admission.run_id()).await.expect("current");
    let selected = opened.test_select_qualified(&current).expect("selected");
    let output = catalog
        .qualify_retained::<BackendValue>(
            context.contract_ref().clone(),
            base.objects()[0].canonical_json().as_bytes(),
        )
        .expect("qualified output")
        .erase();
    let selected = match second.history_port().prepare_selected_pure_conclusion(
        selected,
        ProposedStateOutcome::Success {
            output,
            facts: FactProposalSet::empty(),
        },
    ) {
        SelectedConclusionPreparationOutcome::Rejected {
            selected,
            error: StoreError::Identity,
        } => selected,
        other => panic!("unexpected foreign selection outcome: {other:?}"),
    };
    let owner = opened
        .test_prepare_conclusion_fixture(
            &current,
            &document,
            &selected,
            1,
            mfm_journal::single_trust::StateConcluded::Pure {
                occurrence: occurrence.clone(),
                outcome: mfm_journal::single_trust::StateOutcome::Success(context.clone()),
                fact_proposals: None,
                fact_publication: None,
            },
            vec![base.objects()[0].clone()],
            mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
        )
        .expect("conclusion owner");
    let owner = match second
        .commit_conclusion(owner)
        .await
        .expect("foreign conclusion rejection")
    {
        ConclusionCommitOutcome::Rejected {
            owner,
            error: StoreError::Identity,
        } => owner,
        other => panic!("unexpected foreign conclusion outcome: {other:?}"),
    };
    let alternate = mfm_journal::single_trust::RunFrame::new(
        owner.frame().run_id().clone(),
        owner.frame().store_scope_id().clone(),
        owner.frame().store_epoch(),
        owner.frame().expected_sequence(),
        AppendRequestId::new("classification-alternate-0123456789").expect("request"),
        owner.frame().record().clone(),
        owner.frame().objects().to_vec(),
    )
    .expect("alternate conclusion");
    assert_eq!(
        opened
            .history_port()
            .append_frame(alternate)
            .await
            .expect("alternate append"),
        AppendDisposition::NewlyCommitted { sequence: 2 }
    );
    let history = match opened.commit_conclusion(owner).await.expect("same retry") {
        ConclusionCommitOutcome::AlreadyConcludedSame { history } => history,
        other => panic!("unexpected same retry: {other:?}"),
    };
    assert_eq!(history.head_sequence(), 2);

    let alternate_json = r#"{"value":2}"#;
    let alternate_content = ContentRef::new(
        context.value_ref().schema_id().clone(),
        raw_content_digest(alternate_json.as_bytes()),
    )
    .expect("alternate value");
    let alternate_context =
        ValueRef::new(context.contract_ref().clone(), alternate_content.clone());
    let alternate_object = mfm_journal::single_trust::ImmutableObject::new(
        StableId::new("mfm.value").expect("object type"),
        alternate_content,
        alternate_json.to_owned(),
    )
    .expect("alternate object");
    let conflicting_owner = opened
        .test_prepare_conclusion_fixture(
            &current,
            &document,
            &selected,
            1,
            mfm_journal::single_trust::StateConcluded::Pure {
                occurrence,
                outcome: mfm_journal::single_trust::StateOutcome::Success(alternate_context),
                fact_proposals: None,
                fact_publication: None,
            },
            vec![base.objects()[0].clone(), alternate_object],
            mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
        )
        .expect("conflicting owner");
    match opened
        .commit_conclusion(conflicting_owner)
        .await
        .expect("conflicting retry")
    {
        ConclusionCommitOutcome::Conflict { history } => assert_eq!(history.head_sequence(), 2),
        other => panic!("unexpected conflicting retry: {other:?}"),
    }
}

#[tokio::test]
async fn conclusion_head_race_classifies_superseded_access_without_reentry() {
    let identity = identity();
    let base = admission_frame(&identity);
    let (entry, contract, context, configuration) = match base.record() {
        RunRecord::RunAdmitted(admitted) => (
            admitted.entry_point_id().clone(),
            admitted.admitted_context().contract_ref().clone(),
            admitted.admitted_context().clone(),
            admitted.configuration().clone(),
        ),
        _ => panic!("expected admission"),
    };
    let occurrence = mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
        .expect("occurrence");
    let implementation_ref = fact_source(82);
    let capability_ref = mfm_program::capability_contract_ref::<BackendRead>().expect("capability");
    let adapter_ref = fact_source(84);
    let binding = mfm_program::BindingDescriptor::new(
        implementation_ref.clone(),
        Some(capability_ref.clone()),
        Some(adapter_ref.clone()),
        fact_source(85),
        None,
        None,
    )
    .expect("binding");
    let binding_ref = binding.content_ref().expect("binding ref");
    let state = mfm_program::single_trust::StateDeclaration::new(
        occurrence.clone(),
        implementation_ref,
        contract.clone(),
        contract.clone(),
        None,
        mfm_program::single_trust::ExecutionMode::Read {
            capability_contract_ref: capability_ref,
            total_attempt_bound: 2,
            fact_selection_required: false,
        },
        true,
    )
    .expect("state")
    .with_execution_binding(binding.clone())
    .expect("execution binding");
    let maximum_conclusion_bytes = state.maximum_conclusion_bytes();
    let document = mfm_program::single_trust::ProgramDocument::new(
        entry,
        contract.clone(),
        contract,
        vec![mfm_program::Declaration::State(Box::new(state))],
    )
    .expect("document");
    let admission = mfm_journal::single_trust::RunFrame::new(
        base.run_id().clone(),
        identity.scope().clone(),
        identity.epoch(),
        1,
        AppendRequestId::new("access-classification-admission-0123456789").expect("request"),
        RunRecord::RunAdmitted(
            mfm_journal::single_trust::RunAdmitted::new(
                identity.scope().clone(),
                identity.epoch(),
                base.run_id().clone(),
                identity.tenant().clone(),
                match base.record() {
                    RunRecord::RunAdmitted(admitted) => admitted.entry_point_id().clone(),
                    _ => unreachable!(),
                },
                document.program_ref().expect("program"),
                context.clone(),
                configuration,
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![base.objects()[0].clone(), program_object(&document)],
    )
    .expect("admission frame");
    let (catalog, _) = backend_catalog_builder()
        .finish(document.clone())
        .expect("catalog");
    let opened =
        StructuredStore::open_memory(identity.clone(), catalog, StoreWorkLimits::default())
            .expect("opened store");
    let configuration_head = configuration_head(&opened).await;
    opened
        .test_append_admission(admission.clone(), &configuration_head)
        .await
        .expect("admission append");
    let current = opened.load(admission.run_id()).await.expect("current");
    let selected = opened
        .test_select_qualified(&current)
        .expect("selected ready");
    let prepared = mfm_journal::single_trust::StatePrepared::new(
        occurrence.clone(),
        0,
        context.clone(),
        context.clone(),
        None,
        None,
        mfm_journal::single_trust::PreparationMode::Read {
            total_attempt_bound: 2,
        },
        binding_ref.clone(),
        None,
        maximum_conclusion_bytes,
    )
    .expect("initial preparation");
    let initial = opened
        .test_prepare_access_fixture(
            &current,
            &document,
            &selected,
            1,
            AppendRequestId::new("access-classification-preparation-0-0123456789")
                .expect("request"),
            prepared,
            vec![base.objects()[0].clone()],
        )
        .await
        .expect("initial preparation append");
    let initial_ref = initial.preparation().cloned().expect("initial ref");
    let prepared_history = opened
        .load(admission.run_id())
        .await
        .expect("prepared history");
    let prepared_reduced = opened
        .test_select_qualified(&prepared_history)
        .expect("prepared reduction");
    let owner = opened
        .test_prepare_conclusion_fixture(
            &prepared_history,
            &document,
            &prepared_reduced,
            2,
            mfm_journal::single_trust::StateConcluded::Access {
                occurrence: occurrence.clone(),
                preparation: initial_ref.clone(),
                evidence: context.clone(),
                outcome: mfm_journal::single_trust::StateOutcome::Success(context.clone()),
                fact_proposals: None,
                fact_selection: None,
                fact_publication: None,
            },
            vec![base.objects()[0].clone()],
            maximum_conclusion_bytes,
        )
        .expect("conclusion owner");
    let replacement = mfm_journal::single_trust::StatePrepared::new(
        occurrence,
        1,
        context.clone(),
        context,
        None,
        None,
        mfm_journal::single_trust::PreparationMode::Read {
            total_attempt_bound: 2,
        },
        binding_ref,
        Some(initial_ref),
        maximum_conclusion_bytes,
    )
    .expect("replacement preparation");
    opened
        .test_prepare_access_fixture(
            &prepared_history,
            &document,
            &prepared_reduced,
            2,
            AppendRequestId::new("access-classification-preparation-1-0123456789")
                .expect("request"),
            replacement,
            vec![base.objects()[0].clone()],
        )
        .await
        .expect("replacement append");
    match opened
        .commit_conclusion(owner)
        .await
        .expect("classification")
    {
        ConclusionCommitOutcome::NoLongerSelected { history } => {
            assert_eq!(history.head_sequence(), 3)
        }
        other => panic!("unexpected superseded conclusion result: {other:?}"),
    }
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn unknown_conclusion_retry_finds_same_frame_without_republishing_facts() {
    let identity = identity();
    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let context_content = ContentRef::new(
        contract.schema_id().clone(),
        raw_content_digest(br#"{"value":1}"#),
    )
    .expect("context");
    let context = ValueRef::new(contract.clone(), context_content.clone());
    let occurrence = mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
        .expect("occurrence");
    let state = mfm_program::single_trust::StateDeclaration::new(
        occurrence.clone(),
        contract.clone(),
        contract.clone(),
        contract.clone(),
        None,
        mfm_program::single_trust::ExecutionMode::Pure,
        true,
    )
    .expect("state");
    let maximum_conclusion_bytes = state.maximum_conclusion_bytes();
    let document = mfm_program::single_trust::ProgramDocument::new(
        StableId::new("mfm.test.unknown-conclusion-entry").expect("entry"),
        contract.clone(),
        contract,
        vec![mfm_program::Declaration::State(Box::new(state))],
    )
    .expect("document");
    let program_ref = document.program_ref().expect("program ref");
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:6123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .expect("run id");
    let admission = mfm_journal::single_trust::RunFrame::new(
        run_id.clone(),
        identity.scope().clone(),
        identity.epoch(),
        1,
        AppendRequestId::new("unknown-conclusion-admission-012345").expect("request"),
        RunRecord::RunAdmitted(
            mfm_journal::single_trust::RunAdmitted::new(
                identity.scope().clone(),
                identity.epoch(),
                run_id.clone(),
                identity.tenant().clone(),
                StableId::new("mfm.test.unknown-conclusion-entry").expect("entry"),
                program_ref,
                context.clone(),
                ConfigurationHeadProjection::new(
                    1,
                    ContentRef::new(
                        BackendConfig::schema_id().expect("configuration schema"),
                        raw_content_digest(br#"{"value":1}"#),
                    )
                    .expect("configuration"),
                )
                .expect("configuration head"),
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![
            mfm_journal::single_trust::ImmutableObject::new(
                StableId::new("mfm.value").expect("object type"),
                context_content,
                r#"{"value":1}"#.to_owned(),
            )
            .expect("context object"),
            program_object(&document),
        ],
    )
    .expect("admission frame");
    let (catalog, _) = backend_catalog_builder()
        .finish(document.clone())
        .expect("catalog");
    let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
    let opened = StructuredStore::open(
        backend.clone(),
        identity,
        catalog,
        StoreWorkLimits::default(),
    )
    .await
    .expect("opened store");
    let configuration = configuration_head(&opened).await;
    opened
        .test_append_admission(admission.clone(), &configuration)
        .await
        .expect("admission");
    let current = opened.load(&run_id).await.expect("current");
    let selected = opened.test_select_qualified(&current).expect("selected");
    let (proposal_value, proposal_object) = proposal(100);
    let owner = opened
        .test_prepare_conclusion_fixture(
            &current,
            &document,
            &selected,
            1,
            mfm_journal::single_trust::StateConcluded::Pure {
                occurrence,
                outcome: mfm_journal::single_trust::StateOutcome::Success(context),
                fact_proposals: Some(proposal_value),
                fact_publication: None,
            },
            vec![admission.objects()[0].clone(), proposal_object],
            maximum_conclusion_bytes,
        )
        .expect("conclusion owner");
    backend.fail_next_history_acknowledgement();
    let owner = match opened
        .commit_conclusion(owner)
        .await
        .expect("unknown commit")
    {
        ConclusionCommitOutcome::AcknowledgementUnknown(owner) => {
            assert_eq!(
                owner
                    .frame()
                    .record()
                    .fact_publication()
                    .map(|publication| publication.publication_sequence()),
                Some(1)
            );
            owner
        }
        other => panic!("unexpected unknown outcome: {other:?}"),
    };
    let (disposition, frame) = match opened
        .commit_conclusion(owner)
        .await
        .expect("unknown resolution")
    {
        ConclusionCommitOutcome::Disposition { disposition, frame } => (disposition, frame),
        other => panic!("unexpected resolution outcome: {other:?}"),
    };
    assert_eq!(disposition, AppendDisposition::Found { sequence: 2 });
    assert_eq!(
        frame
            .record()
            .fact_publication()
            .map(|publication| publication.publication_sequence()),
        Some(1)
    );
    let retained = opened.load(&run_id).await.expect("retained");
    let facts = opened.split().into_parts().3.facts().await.expect("facts");
    assert_eq!(facts.head_sequence(), 1);
    assert_eq!(facts.publications().len(), 1);
    assert_eq!(retained.head_sequence(), 2);
}

#[tokio::test]
async fn configuration_heads_cannot_cross_store_openings() {
    let identity = identity();
    let frame = admission_frame(&identity);
    let contract = match frame.record() {
        mfm_journal::single_trust::RunRecord::RunAdmitted(admitted) => {
            admitted.admitted_context().contract_ref().clone()
        }
        _ => panic!("expected admission"),
    };
    let (catalog, _) = backend_catalog_builder()
        .finish(
            mfm_program::single_trust::ProgramDocument::new(
                mfm_ids::StableId::new("mfm.test.configuration-brand-entry").expect("entry"),
                contract.clone(),
                contract,
                Vec::new(),
            )
            .expect("document"),
        )
        .expect("catalog");
    let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
    let first = StructuredStore::open(
        backend.clone(),
        identity.clone(),
        catalog.clone(),
        StoreWorkLimits::default(),
    )
    .await
    .expect("first opening");
    let second = StructuredStore::open(backend, identity, catalog, StoreWorkLimits::default())
        .await
        .expect("second opening");
    let first_configuration = first.test_configuration();
    let second_configuration = second.test_configuration();
    let first_head = configuration_head(&first).await;
    let second_head = second_configuration
        .load::<BackendConfig>()
        .await
        .expect("second resolved")
        .into_head();
    assert!(matches!(
        first_configuration.write_session::<BackendConfig>(&second_head),
        Err(StoreError::Identity)
    ));
    assert!(matches!(
        second_configuration.write_session::<BackendConfig>(&first_head),
        Err(StoreError::Identity)
    ));
}

#[tokio::test]
async fn configuration_owner_promotes_direct_success_and_retains_stale_owner() {
    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let document = mfm_program::single_trust::ProgramDocument::new(
        mfm_ids::StableId::new("mfm.test.configuration-entry").expect("entry"),
        contract.clone(),
        contract,
        Vec::new(),
    )
    .expect("document");
    let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
    let opened = StructuredStore::open_memory(identity(), catalog, StoreWorkLimits::default())
        .expect("opened store");
    let configuration = opened.test_configuration();
    let owner = configuration
        .initial_write_session::<BackendConfig>()
        .prepare_local(
            AppendRequestId::new("configuration-direct-0123456789ab").expect("request"),
            ValidatedConfig::new(BackendConfig { value: 1 }).expect("config"),
        )
        .expect("direct owner");
    let stale_owner = configuration
        .initial_write_session::<BackendConfig>()
        .prepare_local(
            AppendRequestId::new("configuration-stale-0123456789ab").expect("request"),
            ValidatedConfig::new(BackendConfig { value: 2 }).expect("config"),
        )
        .expect("stale owner");
    let promoted = match configuration.commit(owner).await.expect("commit") {
        ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
        other => panic!("unexpected direct outcome: {other:?}"),
    };
    assert_eq!(promoted.head().sequence(), 1);
    assert_eq!(promoted.value().value, 1);
    match configuration
        .commit(stale_owner)
        .await
        .expect("stale result")
    {
        ConfigurationCommitOutcome::StaleHead { actual_sequence: 1 } => {}
        other => panic!("unexpected stale outcome: {other:?}"),
    }
    let conflict = configuration
        .initial_write_session::<BackendConfig>()
        .prepare_local(
            AppendRequestId::new("configuration-direct-0123456789ab").expect("request"),
            ValidatedConfig::new(BackendConfig { value: 9 }).expect("config"),
        )
        .expect("conflict owner");
    assert!(matches!(
        configuration
            .commit(conflict)
            .await
            .expect("conflict result"),
        ConfigurationCommitOutcome::Rejected {
            error: StoreError::Conflict,
            ..
        }
    ));
    let restarted = configuration
        .load::<BackendConfig>()
        .await
        .expect("restart load");
    assert_eq!(restarted.value().value, 1);
}

#[tokio::test]
async fn typed_configuration_ingress_counts_real_decode_and_validation_boundaries() {
    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let document = mfm_program::single_trust::ProgramDocument::new(
        mfm_ids::StableId::new("mfm.test.configuration-counted-entry").expect("entry"),
        contract.clone(),
        contract,
        Vec::new(),
    )
    .expect("document");
    let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
    let opened = StructuredStore::open_memory(identity(), catalog, StoreWorkLimits::default())
        .expect("store");
    let configuration = opened.test_configuration();

    CONFIG_DECODES.store(0, Ordering::SeqCst);
    CONFIG_VALIDATIONS.store(0, Ordering::SeqCst);
    let local = ValidatedConfig::new(CountedConfig { value: 7 }).expect("valid local");
    let owner = configuration
        .initial_write_session::<CountedConfig>()
        .prepare_local(
            AppendRequestId::new("configuration-counted-local-00001").expect("request"),
            local,
        )
        .expect("local owner");
    assert_eq!(CONFIG_DECODES.load(Ordering::SeqCst), 0);
    assert_eq!(CONFIG_VALIDATIONS.load(Ordering::SeqCst), 1);
    let resolved = match configuration.commit(owner).await.expect("commit") {
        ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
        other => panic!("unexpected local outcome: {other:?}"),
    };
    assert_eq!(CONFIG_DECODES.load(Ordering::SeqCst), 0);
    assert_eq!(CONFIG_VALIDATIONS.load(Ordering::SeqCst), 1);

    CONFIG_DECODES.store(0, Ordering::SeqCst);
    CONFIG_VALIDATIONS.store(0, Ordering::SeqCst);
    let loaded = configuration
        .load::<CountedConfig>()
        .await
        .expect("retained load");
    assert_eq!(loaded.value().value, 7);
    assert_eq!(CONFIG_DECODES.load(Ordering::SeqCst), 1);
    assert_eq!(CONFIG_VALIDATIONS.load(Ordering::SeqCst), 1);

    let retry = configuration
        .initial_write_session::<CountedConfig>()
        .prepare_local(
            AppendRequestId::new("configuration-counted-local-00001").expect("request"),
            ValidatedConfig::new(CountedConfig { value: 7 }).expect("valid retry"),
        )
        .expect("retry owner");
    CONFIG_DECODES.store(0, Ordering::SeqCst);
    CONFIG_VALIDATIONS.store(0, Ordering::SeqCst);
    match configuration.commit(retry).await.expect("retry") {
        ConfigurationCommitOutcome::Found(found) => assert_eq!(found.value().value, 7),
        other => panic!("unexpected found outcome: {other:?}"),
    }
    assert_eq!(CONFIG_DECODES.load(Ordering::SeqCst), 1);
    assert_eq!(CONFIG_VALIDATIONS.load(Ordering::SeqCst), 1);

    let external = configuration
        .write_session::<CountedConfig>(resolved.head())
        .expect("external session");
    CONFIG_DECODES.store(0, Ordering::SeqCst);
    CONFIG_VALIDATIONS.store(0, Ordering::SeqCst);
    let external = external
        .prepare_external(
            AppendRequestId::new("configuration-counted-external-01").expect("request"),
            br#"{"value":8}"#,
        )
        .expect("external owner");
    assert_eq!(CONFIG_DECODES.load(Ordering::SeqCst), 1);
    assert_eq!(CONFIG_VALIDATIONS.load(Ordering::SeqCst), 1);
    assert!(matches!(
        configuration
            .commit(external)
            .await
            .expect("external commit"),
        ConfigurationCommitOutcome::NewlyCommitted(_)
    ));
    assert_eq!(CONFIG_DECODES.load(Ordering::SeqCst), 1);
    assert_eq!(CONFIG_VALIDATIONS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn typed_load_selects_latest_type_at_the_captured_global_head() {
    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let document = mfm_program::single_trust::ProgramDocument::new(
        mfm_ids::StableId::new("mfm.test.configuration-heterogeneous-entry").expect("entry"),
        contract.clone(),
        contract,
        Vec::new(),
    )
    .expect("document");
    let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
    let opened = StructuredStore::open_memory(identity(), catalog, StoreWorkLimits::default())
        .expect("store");
    let configuration = opened.test_configuration();
    let first = configuration
        .initial_write_session::<BackendConfig>()
        .prepare_local(
            AppendRequestId::new("configuration-heterogeneous-first-01").expect("request"),
            ValidatedConfig::new(BackendConfig { value: 1 }).expect("config"),
        )
        .expect("first owner");
    let first = match configuration.commit(first).await.expect("first commit") {
        ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
        other => panic!("unexpected first outcome: {other:?}"),
    };
    let second = configuration
        .write_session::<StringConfig>(first.head())
        .expect("second session")
        .prepare_local(
            AppendRequestId::new("configuration-heterogeneous-second-1").expect("request"),
            ValidatedConfig::new(StringConfig {
                value: "two".to_owned(),
            })
            .expect("config"),
        )
        .expect("second owner");
    assert!(matches!(
        configuration.commit(second).await.expect("second commit"),
        ConfigurationCommitOutcome::NewlyCommitted(_)
    ));

    let loaded = configuration
        .load::<BackendConfig>()
        .await
        .expect("typed load");
    assert_eq!(loaded.value().value, 1);
    assert_eq!(loaded.head().sequence, 1);
    assert_eq!(loaded.head().global_sequence, 2);
    let session = configuration
        .write_session::<BackendConfig>(loaded.head())
        .expect("successor session");
    assert_eq!(session.expected_sequence, 2);
}

#[tokio::test]
async fn external_configuration_ingress_rejects_noncanonical_float_secret_and_oversize() {
    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let document = mfm_program::single_trust::ProgramDocument::new(
        mfm_ids::StableId::new("mfm.test.configuration-negative-entry").expect("entry"),
        contract.clone(),
        contract,
        Vec::new(),
    )
    .expect("document");
    let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
    let opened = StructuredStore::open_memory(identity(), catalog, StoreWorkLimits::default())
        .expect("store");
    let configuration = opened.test_configuration();
    assert!(matches!(
        configuration
            .initial_write_session::<CountedConfig>()
            .prepare_external(
                AppendRequestId::new("configuration-noncanonical-00001").expect("request"),
                br#"{"value": 1}"#,
            ),
        Err(StoreError::InvalidRecord)
    ));
    assert!(matches!(
        configuration
            .initial_write_session::<CountedConfig>()
            .prepare_external(
                AppendRequestId::new("configuration-float-00000000001").expect("request"),
                br#"{"value":1.0}"#,
            ),
        Err(StoreError::InvalidRecord)
    ));
    assert!(matches!(
        configuration
            .initial_write_session::<StringConfig>()
            .prepare_external(
                AppendRequestId::new("configuration-secret-0000000001").expect("request"),
                br#"{"value":"secret"}"#,
            ),
        Err(StoreError::InvalidRecord)
    ));
    let oversized = vec![b' '; mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES + 1];
    assert!(matches!(
        configuration
            .initial_write_session::<StringConfig>()
            .prepare_external(
                AppendRequestId::new("configuration-oversized-0000001").expect("request"),
                &oversized,
            ),
        Err(StoreError::Capacity)
    ));
}

#[tokio::test]
async fn configuration_capacity_accepts_each_exact_bound_and_rejects_plus_one() {
    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let document = mfm_program::single_trust::ProgramDocument::new(
        mfm_ids::StableId::new("mfm.test.configuration-count-entry").expect("entry"),
        contract.clone(),
        contract,
        Vec::new(),
    )
    .expect("document");
    let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
    let count_store = StructuredStore::open_memory(identity(), catalog, StoreWorkLimits::default())
        .expect("count store");
    let count_configuration = count_store.test_configuration();
    let mut count_head = None;
    for ordinal in 0..mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS {
        let session = match &count_head {
            Some(head) => count_configuration
                .write_session::<BackendConfig>(head)
                .expect("count session"),
            None => count_configuration.initial_write_session::<BackendConfig>(),
        };
        let owner = session
            .prepare_local(
                AppendRequestId::new(format!("configuration-count-{ordinal:04}-0000000000"))
                    .expect("request"),
                ValidatedConfig::new(BackendConfig {
                    value: ordinal as u64,
                })
                .expect("config"),
            )
            .expect("count owner");
        let resolved = match count_configuration.commit(owner).await.expect("commit") {
            ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
            other => panic!("unexpected count outcome: {other:?}"),
        };
        count_head = Some(resolved.into_head());
    }
    assert_eq!(
        count_head.as_ref().expect("count head").sequence(),
        mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS as u64
    );
    assert!(matches!(
        count_configuration
            .write_session::<BackendConfig>(count_head.as_ref().expect("count head"))
            .expect("final count session")
            .prepare_local(
                AppendRequestId::new("configuration-plus-one-count-000001").expect("request"),
                ValidatedConfig::new(BackendConfig { value: 1024 }).expect("config"),
            ),
        Err(StoreError::Capacity)
    ));
    exercise_typed_configuration_byte_capacity().await;
    eprintln!(
        "capacity-envelope configuration revisions={} revision_bytes={} stream_bytes={}",
        mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS,
        mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES,
        mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES,
    );
}

async fn exercise_typed_configuration_byte_capacity() {
    fn exact_json(bytes: usize) -> Vec<u8> {
        const OVERHEAD: usize = 12;
        format!(r#"{{"value":"{}"}}"#, "a".repeat(bytes - OVERHEAD)).into_bytes()
    }

    let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
    let document = mfm_program::single_trust::ProgramDocument::new(
        mfm_ids::StableId::new("mfm.test.configuration-capacity-entry").expect("entry"),
        contract.clone(),
        contract,
        Vec::new(),
    )
    .expect("document");
    let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
    let opened = StructuredStore::open_memory(identity(), catalog, StoreWorkLimits::default())
        .expect("store");
    let configuration = opened.test_configuration();
    let exact = exact_json(mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES);
    let mut head = None;
    for ordinal in 0..4 {
        let session = match &head {
            Some(head) => configuration
                .write_session::<StringConfig>(head)
                .expect("successor session"),
            None => configuration.initial_write_session::<StringConfig>(),
        };
        let owner = session
            .prepare_external(
                AppendRequestId::new(format!("configuration-exact-stream-{ordinal}-000000"))
                    .expect("request"),
                &exact,
            )
            .expect("exact revision");
        let resolved = match configuration.commit(owner).await.expect("commit") {
            ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
            other => panic!("unexpected capacity outcome: {other:?}"),
        };
        assert_eq!(resolved.canonical_json().len(), exact.len());
        head = Some(resolved.into_head());
    }
    let head = head.expect("stream head");
    assert!(matches!(
        configuration
            .write_session::<StringConfig>(&head)
            .expect("session")
            .prepare_local(
                AppendRequestId::new("configuration-plus-one-stream-0001").expect("request"),
                ValidatedConfig::new(StringConfig {
                    value: "a".to_owned(),
                })
                .expect("config"),
            ),
        Err(StoreError::Capacity)
    ));

    let plus_one = vec![b'a'; mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES + 1];
    assert!(matches!(
        configuration
            .initial_write_session::<StringConfig>()
            .prepare_external(
                AppendRequestId::new("configuration-plus-one-revision-01").expect("request"),
                &plus_one,
            ),
        Err(StoreError::Capacity)
    ));
}

#[tokio::test]
async fn retained_configuration_rejects_wrong_type_digest_and_legacy_schema() {
    async fn opened_with_raw(
        content_ref: ContentRef,
    ) -> (OpenedStructuredStore, Arc<MemoryStructuredBackend>) {
        let identity = identity();
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        let request = AppendRequestId::new("configuration-retained-invalid-001").expect("request");
        let command = ConfigurationAppendCommand::new(
            &identity,
            0,
            &request,
            br#"{"value":1}"#,
            &content_ref,
        );
        assert_eq!(
            backend
                .compare_and_append_configuration(&command)
                .await
                .expect("raw append"),
            BackendConfigurationOutcome::NewlyCommitted
        );
        let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
        let document = mfm_program::single_trust::ProgramDocument::new(
            mfm_ids::StableId::new("mfm.test.configuration-retained-entry").expect("entry"),
            contract.clone(),
            contract,
            Vec::new(),
        )
        .expect("document");
        let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
        let opened = StructuredStore::open(
            backend.clone(),
            identity,
            catalog,
            StoreWorkLimits::default(),
        )
        .await
        .expect("store");
        (opened, backend)
    }

    let wrong_type = ContentRef::new(
        BackendValue::schema_id().expect("wrong schema"),
        raw_content_digest(br#"{"value":1}"#),
    )
    .expect("wrong type");
    let (opened, _) = opened_with_raw(wrong_type).await;
    assert!(matches!(
        opened.test_configuration().load::<CountedConfig>().await,
        Err(StoreError::InvalidRecord)
    ));

    let wrong_digest = ContentRef::new(
        CountedConfig::schema_id().expect("schema"),
        raw_content_digest(b"different"),
    )
    .expect("wrong digest");
    let (opened, _) = opened_with_raw(wrong_digest).await;
    assert!(matches!(
        opened.test_configuration().load::<CountedConfig>().await,
        Err(StoreError::InvalidHistory)
    ));

    let legacy_schema = SchemaId::new(
        "mfm.configuration",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .expect("legacy schema");
    let legacy =
        ContentRef::new(legacy_schema, raw_content_digest(br#"{"value":1}"#)).expect("legacy ref");
    let (opened, _) = opened_with_raw(legacy).await;
    assert!(matches!(
        opened.test_configuration().load::<CountedConfig>().await,
        Err(StoreError::InvalidHistory)
    ));
}
