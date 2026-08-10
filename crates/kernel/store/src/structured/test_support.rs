//! Minimal verified fixtures shared by cross-crate contract tests.
//!
//! This module is enabled only by the `test-support` feature. It deliberately
//! returns the same opaque export evidence that production readers return; it
//! does not expose a second reducer or a way to construct production authority.
//!
//! Program trust here is real: every fixture assembles a qualified registry and
//! certifies an authored program through the certification owner, so there is
//! no permissive verifier and no synthetic certification document.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use mfm_canonical::sha256_digest_bytes;
use mfm_certify::structured::{
    CertifiedProgramRegistry as CertifiedAssemblyRegistry, ProgramRegistryBuilder,
};
use mfm_facts::{
    CanonicalFactPredicate, FactOrdering, FactProposal, FactSelectionLimit, FactSelectionQuery,
    FactSelectionReadResponse, FactSelectionRequest, FactSelectionScanBounds, FactSet,
    FactTieBreak, ProposedFactValue,
};
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, InvocationIdentity, RunId,
    StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    HistoryObject, PriorRunFactSourceManifest, PriorRunFactSourceRule,
    ADMISSION_CONFIGURATION_OBJECT_TYPE, ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
    ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_program::structured::{
    state_contract, Direct, Never, OperationBuilder, PriorRunFactSelectionCapability,
    ProposedSuccessOutcome, Pure, Read, SafeFailureNotApplicable, SafeFailureSuccessOnly, State,
    StateFrame, StateSettlement, StructuredStateCallbacks,
};
use mfm_program_derive::{MfmValue, PersistedSchema};
use mfm_runtime::history::{
    ProposedCanonicalValue, StructuredAdmissionCommand, StructuredAdmissionMaterial,
};
use mfm_spec::structured::ProposedStateOutcome;
use mfm_spec::structured::{
    AuthoredStructuredProgram, CertifiedProgramDocument, SecretFreeExecutableIdentity,
    SecretFreeImplementationDescriptor, SecretFreeQualificationArtifact, StructuredComponentKind,
    StructuredExpansionProfile,
};
use serde::{Deserialize, Serialize};

use super::memory::StructuredMemoryBackend;
use super::purpose::{ExportRunEvidence, RecordedRunEvidence};
use super::qualification::StructuredStoreError;
use super::qualification::{
    PhysicalBindingAuthorization, PhysicalBindingSupersession, PhysicalObligationChecker,
    ProgramVerificationRegistry,
};
use super::{PhysicalTargetIdentity as StorePhysicalTargetIdentity, StructuredStoreIdentity};
use mfm_values::{CanonicalJsonPersistedSchema, PersistedObjectPayload};

static FACT_SCAN_COUNTERS: OnceLock<Mutex<BTreeMap<String, FactScanCounters>>> = OnceLock::new();

/// Test-only counters for one bounded prior-run fact scan.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FactScanCounters {
    /// Number of top-level scan invocations.
    pub invocations: u64,
    /// Number of dense publication-page backend reads.
    pub publication_pages: u64,
    /// Number of producer-prefix backend reads.
    pub producer_prefix_loads: u64,
    /// Number of retained producer batches reduced.
    pub fold_batches: u64,
    /// Maximum dense publication pages in one scan invocation.
    pub maximum_publication_pages: u64,
    /// Maximum producer-prefix loads in one scan invocation.
    pub maximum_producer_prefix_loads: u64,
    /// Maximum producer batches reduced in one scan invocation.
    pub maximum_fold_batches: u64,
}

/// Resets test-only prior-run fact-scan counters.
#[doc(hidden)]
pub fn reset_fact_scan_counters(store_scope_id: &StoreScopeId) {
    counters()
        .lock()
        .expect("fact scan counter mutex poisoned")
        .insert(
            store_scope_id.as_str().to_owned(),
            FactScanCounters::default(),
        );
}

/// Reads test-only prior-run fact-scan counters.
#[doc(hidden)]
pub fn fact_scan_counters(store_scope_id: &StoreScopeId) -> FactScanCounters {
    counters()
        .lock()
        .expect("fact scan counter mutex poisoned")
        .get(store_scope_id.as_str())
        .copied()
        .unwrap_or_default()
}

fn counters() -> &'static Mutex<BTreeMap<String, FactScanCounters>> {
    FACT_SCAN_COUNTERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn update_counters(scope: &str, update: impl FnOnce(&mut FactScanCounters)) {
    let mut values = counters().lock().expect("fact scan counter mutex poisoned");
    update(values.entry(scope.to_owned()).or_default());
}

pub(super) fn count_fact_scan_invocation(scope: &str) {
    update_counters(scope, |value| {
        value.invocations = value.invocations.saturating_add(1);
    });
}

pub(super) fn count_fact_scan_publication_page(scope: &str) {
    update_counters(scope, |value| {
        value.publication_pages = value.publication_pages.saturating_add(1);
    });
}

pub(super) fn count_fact_scan_producer_prefix_load(scope: &str) {
    update_counters(scope, |value| {
        value.producer_prefix_loads = value.producer_prefix_loads.saturating_add(1);
    });
}

pub(super) fn count_fact_scan_fold_batches(scope: &str, count: usize) {
    let Ok(count) = u64::try_from(count) else {
        return;
    };
    update_counters(scope, |value| {
        value.fold_batches = value.fold_batches.saturating_add(count);
    });
}

pub(super) fn record_fact_scan_maxima(
    scope: &str,
    publication_pages: u64,
    producer_prefix_loads: u64,
    fold_batches: u64,
) {
    update_counters(scope, |value| {
        value.maximum_publication_pages = value.maximum_publication_pages.max(publication_pages);
        value.maximum_producer_prefix_loads = value
            .maximum_producer_prefix_loads
            .max(producer_prefix_loads);
        value.maximum_fold_batches = value.maximum_fold_batches.max(fold_batches);
    });
}

/// The one fixture value carried through every fixture program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.store.fixture",
    name = "value",
    version = "1",
    schema = "mfm.store.fixture.value"
)]
struct FixtureValue {
    value: u64,
}

macro_rules! admission_owner {
    ($name:ident, $schema:literal, $object_type:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
        #[serde(deny_unknown_fields)]
        #[mfm(schema = $schema, version = "1")]
        struct $name {
            discriminator: u8,
        }

        impl PersistedObjectPayload for $name {
            fn object_type() -> mfm_values::Result<StableId> {
                StableId::new($object_type)
                    .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
            }
        }
    };
}

admission_owner!(
    FixtureConfiguration,
    "mfm.store.fixture.admission-configuration",
    ADMISSION_CONFIGURATION_OBJECT_TYPE
);
admission_owner!(
    FixtureContextManifest,
    "mfm.store.fixture.admission-context",
    ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE
);
admission_owner!(
    FixtureRoutingPolicy,
    "mfm.store.fixture.admission-routing",
    ADMISSION_ROUTING_POLICY_OBJECT_TYPE
);

/// A pure state that copies its input forward.
struct CopyState;

impl State for CopyState {
    type Input = FixtureValue;
    type Output = FixtureValue;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.store.fixture/copy-state")
    }
}

/// A pure state that copies its input forward and publishes it as one fact.
struct FactCopyState;

impl State for FactCopyState {
    type Input = FixtureValue;
    type Output = FixtureValue;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.store.fixture/fact-copy-state")
    }

    fn fact_slots() -> mfm_program::Result<Vec<mfm_spec::CertifiedFactSlot>> {
        let contract = mfm_spec::structured::structured_value_contract::<FixtureValue>()?;
        let descriptor = fixture_fact_descriptor()?;
        let descriptor_ref = persisted_ref(&descriptor)?;
        Ok(vec![mfm_spec::CertifiedFactSlot::new(
            0,
            1,
            1,
            descriptor_ref,
            contract.clone(),
            contract,
        )?])
    }
}

/// A Read state whose authorized attempt and observation are real appends.
///
/// Prior-run fact selection is the one Runtime read capability, so a real
/// certified Read state uses its exact reserved protocol.
struct ReadCopyState;

impl State for ReadCopyState {
    type Input = FixtureValue;
    type Output = FixtureValue;
    type Failure = Never;
    type Request = FactSelectionRequest;
    type Returned = FactSelectionReadResponse;
    type SafeFailure = mfm_facts::FactSelectionReadFailure;
    type Execution = Read<PriorRunFactSelectionCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.store.fixture/read-copy-state")
    }
}

/// Encoder-view consumer for fixture inspection only.
struct ExportFixtureConsumer;

impl mfm_authority_seal::ExportEncoderConsumerSeal for ExportFixtureConsumer {}

/// Physical-binding verifier for the fixture's target-fixed retained history.
struct AcceptPhysicalBindings;

impl mfm_authority_seal::PhysicalBindingVerifierSeal for AcceptPhysicalBindings {}

impl PhysicalObligationChecker for AcceptPhysicalBindings {
    fn verify_retained_authorization(
        &self,
        _context: &PhysicalBindingAuthorization<'_>,
        _certificate: &HistoryObject,
    ) -> Result<(), StructuredStoreError> {
        Ok(())
    }

    fn verify_current_authorization(
        &self,
        _context: &PhysicalBindingAuthorization<'_>,
        _certificate: &HistoryObject,
    ) -> Result<(), StructuredStoreError> {
        Ok(())
    }

    fn verify_retained_supersession(
        &self,
        _context: &PhysicalBindingSupersession<'_>,
        _public_lineage_head: &HistoryObject,
        _evidence: &HistoryObject,
    ) -> Result<(), StructuredStoreError> {
        Ok(())
    }

    fn verify_current_supersession(
        &self,
        _context: &PhysicalBindingSupersession<'_>,
        _public_lineage_head: &HistoryObject,
        _evidence: &HistoryObject,
    ) -> Result<(), StructuredStoreError> {
        Ok(())
    }
}

/// One real admitted run plus the exact trust needed to verify its export offline.
pub struct OfflineExportFixture {
    /// Opaque export evidence loaded through the production purpose reader.
    pub export: ExportRunEvidence,
    /// The same run loaded through the recorded-replay purpose reader.
    pub recorded: RecordedRunEvidence,
    /// Program trust used by store and isolated replay reduction.
    pub program_qualifier: Arc<ProgramVerificationRegistry>,
    /// Physical-binding trust used by store and isolated replay reduction.
    physical_binding_verifier: AcceptPhysicalBindings,
}

impl OfflineExportFixture {
    /// Returns the authorized source runs this export carries, so a caller can
    /// seal it with one authorization decision per source.
    pub fn authorized_source_run_ids(&self) -> Vec<RunId> {
        self.export
            .with_encoder_view(ExportFixtureConsumer, |view| {
                view.authorized_source_prefixes()
                    .map(|source| source.run_id().clone())
                    .collect()
            })
    }

    /// Returns the fixture's opaque physical-binding trust for replay tests.
    pub fn physical_binding_verifier(&self) -> &dyn PhysicalObligationChecker {
        &self.physical_binding_verifier
    }

    /// Moves the fixture pieces into an isolated replay test without borrowing the fixture after
    /// its export evidence has been sealed into an authorization closure.
    pub fn into_replay_parts(
        self,
    ) -> (
        ExportRunEvidence,
        RecordedRunEvidence,
        Arc<ProgramVerificationRegistry>,
        Box<dyn PhysicalObligationChecker>,
    ) {
        (
            self.export,
            self.recorded,
            self.program_qualifier,
            Box::new(self.physical_binding_verifier),
        )
    }
}

/// Builds one real admitted run whose semantic frontier is its zero-state completion.
pub async fn zero_state_export(discriminator: u8) -> super::Result<OfflineExportFixture> {
    let fixture = copy_program_fixture(discriminator);
    let backend = StructuredMemoryBackend::new(store_identity(discriminator));
    let opened = super::qualify_and_open_structured_store(
        backend,
        fixture.registry,
        Arc::new(AcceptPhysicalBindings),
    )
    .await?;
    let (run_id, _) = opened
        .runtime
        .admit_run(admission(&fixture.program, discriminator, "root"))
        .await
        .map_err(|_| StructuredStoreError::InvalidHistory)?;
    let export = opened.export_reader.load_for_export(&run_id).await?;
    let recorded = opened
        .replay_reader
        .load_for_recorded_verify(&run_id)
        .await?;
    Ok(OfflineExportFixture {
        export,
        recorded,
        program_qualifier: fixture.program_qualifier,
        physical_binding_verifier: AcceptPhysicalBindings,
    })
}

/// Builds one real read authorization/observation suffix after admission.
///
/// The returned export remains semantically rooted at admission while its
/// physical journal contains the later authorization and observation batches.
/// Replay tests use it to prove that an audit export accepts the suffix and a
/// semantic export rejects carrying it past the semantic cutoff.
pub async fn observed_read_export(discriminator: u8) -> super::Result<OfflineExportFixture> {
    let fixture = fact_scan_fixture(discriminator);
    let backend = StructuredMemoryBackend::new(store_identity(discriminator));
    let opened = super::qualify_and_open_structured_store(
        backend,
        fixture.registry,
        Arc::new(AcceptPhysicalBindings),
    )
    .await?;

    // The producer publishes one real fact so the consumer's selection barrier
    // is covered by an actual dense publication.
    let (producer_run, _) = opened
        .runtime
        .admit_run(admission(&fixture.producer, discriminator, "producer"))
        .await
        .map_err(|_| StructuredStoreError::InvalidHistory)?;
    opened
        .runtime
        .drive_once(&producer_run)
        .await
        .map_err(|_| StructuredStoreError::InvalidHistory)?;

    let (consumer_run, _) = opened
        .runtime
        .admit_run(admission_with_sources(
            &fixture.consumer,
            discriminator,
            "consumer",
            fixture.source_manifest.clone(),
        ))
        .await
        .map_err(|_| StructuredStoreError::InvalidHistory)?;
    opened
        .runtime
        .drive_once(&consumer_run)
        .await
        .map_err(|_| StructuredStoreError::InvalidHistory)?;

    // A consumer that selected prior-run facts must carry its producer prefix,
    // so the export is sealed with that exact authorized source.
    let export = opened.export_reader.load_for_export(&consumer_run).await?;
    let producer_export = opened.export_reader.load_for_export(&producer_run).await?;
    let export = export.with_authorized_sources(None, vec![(producer_export, None)])?;
    let recorded = opened
        .replay_reader
        .load_for_recorded_verify(&consumer_run)
        .await?;
    Ok(OfflineExportFixture {
        export,
        recorded,
        program_qualifier: fixture.program_qualifier,
        physical_binding_verifier: AcceptPhysicalBindings,
    })
}

/// One live memory-backed store over a real certified one-state program.
///
/// Hostile-history tests admit a genuine run through this store, read its exact
/// raw prefix from the backend, forge that prefix, and requalify it. Program
/// trust is the real certification owner, so a forged prefix is rejected by the
/// same rule production uses.
pub struct LiveFixtureStore {
    /// Exact store identity the backend was opened with.
    pub identity: StructuredStoreIdentity,
    /// Backend handle for reading and injecting raw history.
    pub backend: StructuredMemoryBackend,
    /// Real program trust for this fixture's certified program.
    pub program_qualifier: Arc<ProgramVerificationRegistry>,
    /// The admitted run's deterministic identity.
    pub run_id: RunId,
    discriminator: u8,
}

impl LiveFixtureStore {
    /// Opens the fixture store without admitting anything.
    pub fn open(discriminator: u8) -> Self {
        let fixture = copy_program_fixture(discriminator);
        let identity = store_identity(discriminator);
        Self {
            run_id: run_id(&fixture.program.entry_point, discriminator),
            backend: StructuredMemoryBackend::new(identity.clone()),
            identity,
            program_qualifier: fixture.program_qualifier,
            discriminator,
        }
    }

    /// Admits this fixture's run and returns its exact candidate digest.
    ///
    /// The writer and reader stay private: a fixture exposes operations, never
    /// the mutation authority itself.
    pub async fn admit(&self, discriminator: u8, label: &str) -> super::Result<ContentDigest> {
        if discriminator != self.discriminator {
            return Err(StructuredStoreError::InvalidHistory);
        }
        let fixture = copy_program_fixture(discriminator);
        let opened = super::qualify_and_open_structured_store(
            self.backend.clone(),
            fixture.registry,
            Arc::new(AcceptPhysicalBindings),
        )
        .await?;
        let (run_id, attempt) = opened
            .runtime
            .admit_run(admission(&fixture.program, discriminator, label))
            .await
            .map_err(|_| StructuredStoreError::InvalidHistory)?;
        if run_id != self.run_id {
            return Err(StructuredStoreError::InvalidHistory);
        }
        attempt
            .committed()
            .map(|batch| batch.candidate_digest.clone())
            .ok_or(StructuredStoreError::InvalidHistory)
    }

    /// Drives the fixture's one pure state through terminal closure.
    pub async fn drive_once(&self) -> super::Result<()> {
        let fixture = copy_program_fixture(self.discriminator);
        let opened = super::qualify_and_open_structured_store(
            self.backend.clone(),
            fixture.registry,
            Arc::new(AcceptPhysicalBindings),
        )
        .await?;
        opened
            .runtime
            .drive_once(&self.run_id)
            .await
            .map(|_| ())
            .map_err(|_| StructuredStoreError::InvalidHistory)
    }

    /// Loads this fixture run's verified state.
    pub async fn load_verified(&self) -> super::Result<super::purpose::OfflineVerifiedRun> {
        super::verify_offline_recorded_history(
            self.raw_prefix().await?,
            self.program_qualifier.as_ref(),
            &AcceptPhysicalBindings,
        )
    }

    /// Reads this fixture run's exact raw persisted prefix.
    pub async fn raw_prefix(&self) -> super::Result<super::backend::RawRunHistory> {
        use super::backend::StructuredHistoryBackend;
        self.backend
            .load_snapshot(&self.run_id)
            .await?
            .history
            .ok_or(StructuredStoreError::InvalidHistory)
    }

    /// Returns the tenant every fixture run is admitted under.
    pub fn tenant_scope_id(&self) -> TenantScopeId {
        fixture_tenant()
    }

    /// Lists one bounded page of current Effect-entry attention.
    pub async fn list_effect_entry_attention(
        &self,
        tenant_scope_id: &TenantScopeId,
        after_run_id: Option<&RunId>,
        limit: u32,
    ) -> super::Result<super::purpose::EffectEntryAttentionPage> {
        let fixture = copy_program_fixture(self.discriminator);
        let opened = super::qualify_and_open_structured_store(
            self.backend.clone(),
            fixture.registry,
            Arc::new(AcceptPhysicalBindings),
        )
        .await?;
        opened
            .effect_entry_attention_reader
            .list_effect_entry_attention(tenant_scope_id, after_run_id, limit)
            .await
    }

    /// Returns physical-binding trust matching this fixture.
    pub fn physical_binding_verifier(&self) -> Arc<dyn PhysicalObligationChecker> {
        Arc::new(AcceptPhysicalBindings)
    }
}

/// Returns program trust that certifies only an unrelated fixture program.
///
/// Any other run's admission fails at the certification owner because its
/// entry point is not process-qualified in this registry.
pub fn unrelated_program_qualifier() -> Arc<ProgramVerificationRegistry> {
    copy_program_fixture(0xfe).program_qualifier
}

/// Assigns a deliberately hostile raw candidate through the real compiler owner.
pub fn assign_hostile_candidate(
    identity: &StructuredStoreIdentity,
    candidate: mfm_journal::structured::CommitCandidate,
) -> super::Result<mfm_journal::structured::CommittedBatch> {
    super::compiler::assign(identity, candidate)
}

/// Qualifies framing and typed ownership without interpreting event semantics.
pub fn qualify_recorded_structure_only(
    raw: super::backend::RawRunHistory,
    programs: &ProgramVerificationRegistry,
) -> super::Result<()> {
    super::qualification::qualify_recorded_history(raw, programs).map(|_| ())
}

/// Replaces admission's well-typed genesis digest with the wrong semantic value.
pub fn forge_wrong_admission_genesis(
    identity: &StructuredStoreIdentity,
    mut raw: super::backend::RawRunHistory,
) -> super::Result<super::backend::RawRunHistory> {
    let original = raw
        .batches
        .pop()
        .ok_or(StructuredStoreError::InvalidHistory)?;
    if !raw.batches.is_empty() {
        return Err(StructuredStoreError::InvalidHistory);
    }
    let mut records = original
        .records
        .iter()
        .map(|assigned| assigned.record.clone())
        .collect::<Vec<_>>();
    let Some(mfm_journal::structured::RunRecord::RunAdmitted(admission)) = records.first_mut()
    else {
        return Err(StructuredStoreError::InvalidHistory);
    };
    admission.genesis_semantic_state_digest = mfm_ids::RunSemanticStateDigest::from_digest(
        mfm_canonical::sha256_digest_bytes(b"mfm.store.test.wrong-genesis.v1"),
    );
    let batch = super::compiler::assign(
        identity,
        mfm_journal::structured::CommitCandidate {
            run_id: raw.run_id.clone(),
            expected_head: None,
            append_request_id: original.append_request_id,
            tenant_fact_coordinate: original.tenant_fact_coordinate,
            records,
            objects: original.objects,
        },
    )?;
    raw.batches.push(batch);
    Ok(raw)
}

/// Proves incremental advancement equals a complete replay at every prefix.
pub fn verify_incremental_reduction_equivalence(
    raw: super::backend::RawRunHistory,
    programs: &ProgramVerificationRegistry,
    physical: &dyn PhysicalObligationChecker,
) -> super::Result<()> {
    let history = super::qualification::qualify_recorded_history(raw.clone(), programs)?;
    let mut incremental = super::reducer::ReducedRunState::empty(&history.context);
    for (index, batch) in history.batches.iter().enumerate() {
        let finalized = super::semantic_open::replay_step(&history, &incremental, batch, physical)?;
        let full = super::semantic_open::verify_qualified(
            super::backend::RawRunHistory {
                run_id: raw.run_id.clone(),
                batches: raw.batches[..=index].to_vec(),
            },
            programs,
            physical,
        )?;
        if finalized.reduced.as_ref() != full.reduced.as_ref()
            || finalized.run_projection.successor() != &full.current_projection()
            || finalized.committed != raw.batches[index]
        {
            return Err(StructuredStoreError::InvalidHistory);
        }
        incremental = *finalized.into_reduced();
    }
    Ok(())
}

/// One certified fixture program document.
struct FixtureProgram {
    entry_point: StableId,
    document: CertifiedProgramDocument,
}

/// One complete live fixture assembly plus callback-free offline trust.
struct Fixture {
    program: FixtureProgram,
    registry: CertifiedAssemblyRegistry,
    program_qualifier: Arc<ProgramVerificationRegistry>,
}

/// One producer that publishes a fact and one consumer that selects it.
struct FactScanFixture {
    producer: FixtureProgram,
    consumer: FixtureProgram,
    source_manifest: HistoryObject,
    registry: CertifiedAssemblyRegistry,
    program_qualifier: Arc<ProgramVerificationRegistry>,
}

/// The one fact descriptor published and selected by the scan fixture.
fn fixture_fact_descriptor() -> mfm_program::Result<mfm_spec::structured::StructuredFactDescriptor>
{
    let contract_ref = mfm_spec::structured::structured_value_contract_ref::<FixtureValue>()?;
    mfm_spec::structured::StructuredFactDescriptor::new(
        stable("mfm.store.fixture/value-fact")?,
        contract_ref.clone(),
        contract_ref,
    )
    .map_err(Into::into)
}

/// The exact fact the producer publishes.
fn fixture_fact_set() -> super::Result<FactSet> {
    let descriptor = fixture_fact_descriptor().map_err(|_| StructuredStoreError::InvalidHistory)?;
    let value = proposed_fact_value()?;
    let descriptor_ref = descriptor
        .content_ref()
        .map_err(|_| StructuredStoreError::InvalidHistory)?;
    Ok(FactSet::one(
        FactProposal::new(0, descriptor_ref, value.clone(), value)
            .map_err(|_| StructuredStoreError::InvalidHistory)?,
    ))
}

fn proposed_fact_value() -> super::Result<ProposedFactValue> {
    let contract = mfm_spec::structured::structured_value_contract::<FixtureValue>()
        .map_err(|_| StructuredStoreError::InvalidHistory)?;
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(r#"{"value":7}"#)
        .map_err(|_| StructuredStoreError::InvalidHistory)?;
    ProposedFactValue::new(
        contract.schema_id().clone(),
        contract.semantic_type_id().clone(),
        contract.role().clone(),
        contract.media_type().clone(),
        contract.evidence_contract_ref().clone(),
        canonical,
    )
    .map_err(|_| StructuredStoreError::InvalidHistory)
}

/// Assembles and certifies the producer/consumer pair in one registry.
fn fact_scan_fixture(discriminator: u8) -> FactScanFixture {
    let producer_entry_point =
        StableId::new(format!("mfm.store.fixture/producer-{discriminator}")).expect("producer id");
    let consumer_entry_point =
        StableId::new(format!("mfm.store.fixture/consumer-{discriminator}")).expect("consumer id");
    let descriptor = fixture_fact_descriptor().expect("fact descriptor");
    let descriptor_ref = descriptor.content_ref().expect("fact descriptor reference");
    let source_manifest = PriorRunFactSourceManifest::new(vec![PriorRunFactSourceRule::new(
        producer_entry_point.clone(),
        Vec::new(),
        vec![descriptor_ref.clone()],
    )
    .expect("producer source rule")])
    .expect("source manifest");
    let source_manifest =
        HistoryObject::from_persisted(&source_manifest).expect("source manifest object");
    let request = fact_selection_request(&source_manifest.content_ref, &descriptor_ref);

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<FixtureValue>()
        .expect("value contract");
    assembly
        .register_fact_descriptor(fixture_fact_descriptor().expect("fact descriptor"))
        .expect("fact descriptor registration");

    let producer_state_ref = state_contract::<FactCopyState>()
        .expect("producer state contract")
        .content_ref()
        .expect("producer state contract reference");
    let producer_descriptor = implementation_descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        producer_state_ref,
        "fact-copy-state",
    );
    assembly
        .register_state::<FactCopyState>(
            producer_descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(|frame: StateFrame<'_, FixtureValue>| {
                    let value = frame.input().clone();
                    ProposedStateOutcome::success_with_facts(
                        value,
                        fixture_fact_set().expect("fixture fact set"),
                    )
                }),
            },
        )
        .expect("producer state registration");

    let consumer_state_ref = state_contract::<ReadCopyState>()
        .expect("consumer state contract")
        .content_ref()
        .expect("consumer state contract reference");
    let consumer_descriptor = implementation_descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        consumer_state_ref,
        "read-copy-state",
    );
    let authored_request = request.clone();
    assembly
        .register_state::<ReadCopyState>(
            consumer_descriptor,
            StructuredStateCallbacks::Read {
                request: Arc::new(move |_| authored_request.clone()),
                settle_returned: Arc::new(|frame: StateFrame<'_, FixtureValue>, _returned| {
                    StateSettlement::Proposed(ProposedStateOutcome::Success(frame.input().clone()))
                }),
                settle_safe_failure: Arc::new(|frame: StateFrame<'_, FixtureValue>, _| {
                    ProposedSuccessOutcome::new(frame.input().clone())
                }),
            },
        )
        .expect("consumer state registration");

    let producer_program = producer_program(producer_entry_point.clone());
    let consumer_program = read_program(consumer_entry_point.clone());
    for (entry_point, authored) in [
        (&producer_entry_point, &producer_program),
        (&consumer_entry_point, &consumer_program),
    ] {
        assembly
            .register_entry_point(
                entry_point.clone(),
                authored.clone(),
                StructuredExpansionProfile {
                    policies: Vec::new(),
                    max_occurrences: 8,
                    max_declarations: 8,
                    max_lanes: 4,
                    max_fan_out_depth: 2,
                    max_branch_depth: 4,
                },
            )
            .expect("entry point");
    }
    let registry = assembly
        .build(&[producer_entry_point.clone(), consumer_entry_point.clone()])
        .expect("qualified registry");
    let program_qualifier = Arc::new(ProgramVerificationRegistry::new(
        registry.admission_verification_registry(),
    ));
    let certify = |entry_point: &StableId, authored: AuthoredStructuredProgram| FixtureProgram {
        entry_point: entry_point.clone(),
        document: registry
            .certifier(entry_point)
            .expect("certifier")
            .certify(authored)
            .expect("certified program")
            .into_document(),
    };
    FactScanFixture {
        producer: certify(&producer_entry_point, producer_program),
        consumer: certify(&consumer_entry_point, consumer_program),
        source_manifest,
        registry,
        program_qualifier,
    }
}

fn producer_program(operation_id: StableId) -> AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<FixtureValue, Never>::new(
        operation_id,
        stable("root").expect("root id"),
    )
    .expect("builder");
    let input = builder
        .input::<FixtureValue>(stable("input").expect("input id"))
        .expect("input root");
    let output = builder
        .root()
        .state::<FactCopyState>(stable("publish").expect("publish label"), &input)
        .expect("fact state")
        .infallible()
        .expect("infallible fact state");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("producer program")
}

/// Assembles and certifies a one-pure-state fixture program.
fn copy_program_fixture(discriminator: u8) -> Fixture {
    let entry_point =
        StableId::new(format!("mfm.store.fixture/copy-{discriminator}")).expect("entry point");
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<FixtureValue>()
        .expect("value contract");
    let state_contract_ref = state_contract::<CopyState>()
        .expect("state contract")
        .content_ref()
        .expect("state contract reference");
    let descriptor = implementation_descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_contract_ref,
        "copy-state",
    );
    assembly
        .register_state::<CopyState>(
            descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(|frame: StateFrame<'_, FixtureValue>| {
                    ProposedStateOutcome::Success(frame.input().clone())
                }),
            },
        )
        .expect("state registration");
    let authored = copy_program(entry_point.clone());
    finish(assembly, entry_point, authored)
}

/// Registers the entry point, builds the registry, and certifies the program.
fn finish(
    mut assembly: ProgramRegistryBuilder,
    entry_point: StableId,
    authored: AuthoredStructuredProgram,
) -> Fixture {
    assembly
        .register_entry_point(
            entry_point.clone(),
            authored.clone(),
            StructuredExpansionProfile {
                policies: Vec::new(),
                max_occurrences: 8,
                max_declarations: 8,
                max_lanes: 4,
                max_fan_out_depth: 2,
                max_branch_depth: 4,
            },
        )
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&entry_point))
        .expect("qualified registry");
    let document = registry
        .certifier(&entry_point)
        .expect("certifier")
        .certify(authored)
        .expect("certified program")
        .into_document();
    let program_qualifier = Arc::new(ProgramVerificationRegistry::new(
        registry.admission_verification_registry(),
    ));
    Fixture {
        program: FixtureProgram {
            entry_point,
            document,
        },
        registry,
        program_qualifier,
    }
}

fn copy_program(operation_id: StableId) -> AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<FixtureValue, Never>::new(
        operation_id,
        stable("root").expect("root id"),
    )
    .expect("builder");
    let input = builder
        .input::<FixtureValue>(stable("input").expect("input id"))
        .expect("input root");
    let output = builder
        .root()
        .state::<CopyState>(stable("copy").expect("copy label"), &input)
        .expect("copy state")
        .infallible()
        .expect("infallible state");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("program")
}

fn read_program(operation_id: StableId) -> AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<FixtureValue, Never>::new(
        operation_id,
        stable("root").expect("root id"),
    )
    .expect("builder");
    let input = builder
        .input::<FixtureValue>(stable("input").expect("input id"))
        .expect("input root");
    let output = builder
        .root()
        .state::<ReadCopyState>(stable("read").expect("read label"), &input)
        .expect("read state")
        .infallible()
        .expect("infallible read state");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("read program")
}

/// The one fixture fact-selection request, authored by the Read state.
fn fact_selection_request(
    source_manifest_ref: &ContentRef,
    descriptor_ref: &ContentRef,
) -> FactSelectionRequest {
    FactSelectionRequest::new(
        source_manifest_ref.clone(),
        FactSelectionScanBounds::new(1, 64, 1_048_576, 16, 65_536).expect("fact scan bounds"),
        vec![FactSelectionQuery::new(
            descriptor_ref.clone(),
            CanonicalFactPredicate::from_canonical_json(br#"{"value":7}"#).expect("fact predicate"),
            None,
            FactOrdering::Ascending,
            FactSelectionLimit::new(4).expect("fact selection limit"),
            FactTieBreak::FactIdentityAscending,
        )
        .expect("fact query")],
    )
    .expect("fact request")
}

fn implementation_descriptor(
    assembly: &mut ProgramRegistryBuilder,
    component_kind: StructuredComponentKind,
    semantic_contract_ref: ContentRef,
    suffix: &str,
) -> SecretFreeImplementationDescriptor {
    let executable_identity_ref = assembly
        .register_executable_identity(SecretFreeExecutableIdentity {
            executable_id: stable("mfm.store.fixture/test-executable").expect("executable"),
        })
        .expect("executable identity");
    let qualification_artifact_ref = assembly
        .register_qualification_artifact(SecretFreeQualificationArtifact {
            qualification_id: stable("mfm.store.fixture/test-qualification")
                .expect("qualification"),
        })
        .expect("qualification artifact");
    SecretFreeImplementationDescriptor {
        component_kind,
        semantic_contract_ref,
        implementation_id: stable(&format!("mfm.store.fixture/{suffix}-implementation"))
            .expect("implementation id"),
        executable_identity_ref,
        qualification_artifact_ref,
    }
}

fn admission(
    fixture: &FixtureProgram,
    discriminator: u8,
    label: &str,
) -> StructuredAdmissionCommand {
    admission_with_sources(
        fixture,
        discriminator,
        label,
        HistoryObject::from_persisted(
            &PriorRunFactSourceManifest::new(Vec::new()).expect("empty source manifest"),
        )
        .expect("source manifest object"),
    )
}

/// Builds an admission whose prior-run source manifest is exactly `sources`.
fn admission_with_sources(
    fixture: &FixtureProgram,
    discriminator: u8,
    label: &str,
    sources: HistoryObject,
) -> StructuredAdmissionCommand {
    StructuredAdmissionCommand::new(
        fixture_tenant(),
        fixture_invocation(),
        fixture.entry_point.clone(),
        fixture.document.clone(),
        admission_material(discriminator, sources),
        vec![ProposedCanonicalValue::from_json(r#"{"value":7}"#).expect("fixture input")],
        AppendRequestId::new(format!("portable-fixture-admit-{label}-{discriminator}"))
            .expect("fixture append id"),
    )
}

fn admission_material(discriminator: u8, sources: HistoryObject) -> StructuredAdmissionMaterial {
    StructuredAdmissionMaterial::new(
        HistoryObject::from_persisted(&FixtureConfiguration { discriminator })
            .expect("configuration object"),
        HistoryObject::from_persisted(&FixtureContextManifest { discriminator })
            .expect("context object"),
        sources,
        HistoryObject::from_persisted(&FixtureRoutingPolicy { discriminator })
            .expect("routing object"),
        Vec::new(),
    )
    .expect("admission material")
}

fn store_identity(discriminator: u8) -> StructuredStoreIdentity {
    StructuredStoreIdentity {
        store_scope_id: StoreScopeId::new(format!(
            "{}{:032x}",
            StoreScopeId::PREFIX,
            discriminator
        ))
        .expect("fixture store scope"),
        store_epoch: StoreEpoch::new(1),
        physical_target: Some(StorePhysicalTargetIdentity {
            target_key: format!("fixture-target-{discriminator}"),
            database_oid: u32::from(discriminator),
            fence_generation: 1,
            release_epoch: 1,
            current_incarnation_ref: ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(&[discriminator, 6]),
            ),
        }),
    }
}

fn fixture_tenant() -> TenantScopeId {
    TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "b".repeat(32)))
        .expect("fixture tenant")
}

fn fixture_invocation() -> InvocationIdentity {
    InvocationIdentity::new("00000000-0000-4000-8000-0000000000f1").expect("fixture invocation")
}

fn run_id(entry_point: &StableId, discriminator: u8) -> RunId {
    mfm_journal::structured::derive_run_id(
        &store_identity(discriminator).store_scope_id,
        &fixture_tenant(),
        entry_point,
        &fixture_invocation(),
    )
    .expect("fixture run id")
}

fn stable(value: &str) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}

fn persisted_ref<T: CanonicalJsonPersistedSchema>(value: &T) -> mfm_program::Result<ContentRef> {
    value
        .content_ref()
        .map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}
