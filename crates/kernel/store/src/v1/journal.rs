use super::*;

#[path = "journal/fold.rs"]
mod fold;
#[path = "journal/history_validation.rs"]
pub(super) mod history_validation;

use fold::JournalFold;
use mfm_certify::CertifiedTypedSpec;
use mfm_ids::ContentRef;
use std::ops::Range;

use super::current_lifecycle::CurrentLifecycleFold;

#[derive(Debug)]
pub(super) struct CommittedJournalBatch {
    pub(super) seq: StreamSeq,
    pub(super) commit_key: CommitKey,
    pub(super) store_commit_order: StoreCommitOrder,
    pub(super) record_range: Range<usize>,
}

pub(super) struct VerifiedJournalObject {
    pub(super) bytes: Vec<u8>,
    pub(super) evidence: ArtifactEvidenceRef,
}

struct BootstrapObject {
    content_ref: ContentRef,
    object: VerifiedJournalObject,
}

/// Affine verifier supplied only while a durable backend loads one requested journal.
///
/// The private requested-run binding prevents a backend from substituting another run. This token
/// is intentionally non-cloneable and must be consumed to mint journal authority.
#[must_use = "a journal load verifier must be consumed by the backend load"]
pub struct JournalLoadVerifier {
    run_id: RunId,
}

impl JournalLoadVerifier {
    pub(super) fn new(run_id: RunId) -> Self {
        Self { run_id }
    }

    /// Returns the exact run the backend must load.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Consumes this verifier and validates records plus exact event-required objects.
    pub fn verify(
        self,
        records: Vec<KernelEventEnvelope>,
        artifact_bytes: ArtifactByteAuthorityMap,
    ) -> Result<CommittedRunJournal> {
        CommittedRunJournal::from_persisted_records(self.run_id, records, artifact_bytes)
    }

    /// Consumes this verifier around authority already loaded by a delegated journal store.
    ///
    /// Delegating backends use this instead of reconstructing raw records. The loaded journal must
    /// belong to the verifier-bound requested run.
    pub fn accept_verified(self, journal: CommittedRunJournal) -> Result<CommittedRunJournal> {
        accept_journal_for_run(&self.run_id, journal)
    }
}

pub(super) fn accept_journal_for_run(
    requested_run_id: &RunId,
    journal: CommittedRunJournal,
) -> Result<CommittedRunJournal> {
    if journal.run_id() != requested_run_id {
        return Err(StoreError::PersistedEventMismatch {
            field: "run_id",
            message: "journal backend returned a journal for a different requested run".to_owned(),
        });
    }
    Ok(journal)
}

/// Store-owned authority for one admitted run's committed journal.
///
/// Construction verifies persisted record identity and order, atomic batch grouping, the private
/// physical fold, and all exact event-required retained objects before returning. The authority is
/// intentionally non-cloneable; semantic verification occurs when [`Self::verify`] binds it to a
/// trusted certified spec.
pub struct CommittedRunJournal {
    run_id: RunId,
    records: Vec<KernelEventEnvelope>,
    fold: JournalFold,
    certified_spec: BootstrapObject,
    certificate: BootstrapObject,
    artifact_requirements: Vec<EventArtifactRequirement>,
    objects: BTreeMap<ArtifactAuthorityKey, VerifiedJournalObject>,
}

impl ExactRetainedObjectResolver for CommittedRunJournal {
    fn resolve_exact<'a>(
        &'a self,
        key: &ArtifactAuthorityKey,
    ) -> Option<(&'a [u8], &'a ArtifactEvidenceRef)> {
        for bootstrap in [&self.certified_spec, &self.certificate] {
            let evidence = &bootstrap.object.evidence;
            if evidence.artifact_id == key.0
                && evidence.evidence_hash().ok().as_ref() == Some(&key.1)
            {
                return Some((bootstrap.object.bytes.as_slice(), evidence));
            }
        }
        self.objects
            .get(key)
            .map(|object| (object.bytes.as_slice(), &object.evidence))
    }
}

impl fmt::Debug for CommittedRunJournal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommittedRunJournal")
            .field("run_id", &self.run_id)
            .field("current_run_sequence", &self.current_run_sequence())
            .field("record_count", &self.records.len())
            .field("required_object_count", &(self.objects.len() + 2))
            .finish_non_exhaustive()
    }
}

impl CommittedRunJournal {
    /// Reconstructs one journal from persisted records and exact retained-object authority.
    ///
    /// The affine load verifier calls this after a backend loads both inputs consistently.
    fn from_persisted_records(
        run_id: RunId,
        records: Vec<KernelEventEnvelope>,
        artifact_bytes: ArtifactByteAuthorityMap,
    ) -> Result<Self> {
        if records.is_empty() {
            return Err(StoreError::RunNotFound { run_id });
        }
        if let Some(record) = records.iter().find(|record| record.run_id() != &run_id) {
            return Err(StoreError::PersistedEventMismatch {
                field: "run_id",
                message: format!(
                    "journal requested run {} but contains record for {}",
                    run_id,
                    record.run_id()
                ),
            });
        }
        let admission = run_admission_root(&records)?;
        let fold = JournalFold::rebuild(&records)?;
        let artifact_requirements = records
            .iter()
            .flat_map(|record| event_artifact_requirements(record.payload()))
            .collect::<Vec<_>>();
        let artifact_bytes =
            retain_loaded_required_authority(&artifact_requirements, artifact_bytes);
        let loaded_objects = verify_journal_objects(artifact_bytes)?;
        let mut objects = retain_required_journal_objects(&artifact_requirements, loaded_objects)?;
        let certified_spec = take_bootstrap_object(
            &mut objects,
            &admission.spec_artifact,
            ArtifactRole::TypedExecutionSpec,
        )?;
        let certificate = take_bootstrap_object(
            &mut objects,
            &admission.certificate_artifact,
            ArtifactRole::TypedSpecCertificate,
        )?;

        Ok(Self {
            run_id,
            records,
            fold,
            certified_spec,
            certificate,
            artifact_requirements,
            objects,
        })
    }

    /// Returns the run id covered by this journal.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the current persisted run sequence.
    ///
    /// This is a current-format observation, not a frozen recoverability head token.
    pub fn current_run_sequence(&self) -> Option<u64> {
        Some(self.fold.current_run_sequence().as_u64())
    }

    /// Returns the retained certified-spec object needed for certification bootstrap.
    pub fn certified_spec_object(&self) -> (&ContentRef, &[u8]) {
        (
            &self.certified_spec.content_ref,
            &self.certified_spec.object.bytes,
        )
    }

    /// Returns the retained certificate object needed for certification bootstrap.
    pub fn certificate_object(&self) -> (&ContentRef, &[u8]) {
        (
            &self.certificate.content_ref,
            &self.certificate.object.bytes,
        )
    }

    pub(super) fn records(&self) -> &[KernelEventEnvelope] {
        &self.records
    }

    pub(super) fn batches(&self) -> &[CommittedJournalBatch] {
        self.fold.batches()
    }

    pub(super) fn object(
        &self,
        artifact_id: &ArtifactId,
        evidence_hash: &ContentDigest,
    ) -> Option<&VerifiedJournalObject> {
        for bootstrap in [&self.certified_spec, &self.certificate] {
            if &bootstrap.object.evidence.artifact_id == artifact_id
                && bootstrap.object.evidence.evidence_hash().ok().as_ref() == Some(evidence_hash)
            {
                return Some(&bootstrap.object);
            }
        }
        self.objects
            .get(&(artifact_id.clone(), evidence_hash.clone()))
    }

    pub(super) fn requires_object(&self, requirement: &EventArtifactRequirement) -> bool {
        self.artifact_requirements
            .iter()
            .any(|required| required == requirement)
    }

    /// Consumes this journal and binds it to one trusted certified spec.
    pub fn verify(self, certified_spec: CertifiedTypedSpec) -> Result<VerifiedRunView> {
        let persisted = certified_spec
            .to_persisted_parts()
            .map_err(|error| StoreError::Identity(error.to_string()))?;
        if self.certified_spec.object.bytes != persisted.spec_bytes() {
            return Err(StoreError::PersistedEventMismatch {
                field: "certified_spec_object",
                message: "certified spec bytes do not match the admitted retained object"
                    .to_owned(),
            });
        }
        if self.certificate.object.bytes != persisted.certificate_bytes() {
            return Err(StoreError::PersistedEventMismatch {
                field: "certificate_object",
                message: "certificate bytes do not match the admitted retained object".to_owned(),
            });
        }
        let admitted_spec_hash = self.fold.spec_hash();
        if certified_spec.spec_hash() != admitted_spec_hash {
            return Err(StoreError::PersistedEventMismatch {
                field: "spec_hash",
                message: "certified spec does not match the admitted journal root".to_owned(),
            });
        }
        let current_lifecycle = Box::new(CurrentLifecycleFold::build(&certified_spec, &self)?);
        Ok(VerifiedRunView {
            journal: self,
            certified_spec,
            current_lifecycle,
        })
    }
}

/// Opaque verified authority shared by runtime, replay, and read services.
///
/// This view owns the sole committed journal and certified spec authority and is intentionally
/// non-cloneable.
pub struct VerifiedRunView {
    journal: CommittedRunJournal,
    certified_spec: CertifiedTypedSpec,
    current_lifecycle: Box<CurrentLifecycleFold>,
}

impl fmt::Debug for VerifiedRunView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedRunView")
            .field("run_id", &self.run_id())
            .field("spec_hash", &self.spec_hash())
            .field("current_run_sequence", &self.current_run_sequence())
            .finish_non_exhaustive()
    }
}

impl VerifiedRunView {
    /// Returns the admitted run id.
    pub fn run_id(&self) -> &RunId {
        self.journal.run_id()
    }

    /// Returns the certified spec hash bound to this view.
    pub fn spec_hash(&self) -> &SpecHash {
        self.certified_spec.spec_hash()
    }

    /// Returns the current persisted run sequence.
    ///
    /// This is a current-format observation, not a frozen recoverability head token.
    pub fn current_run_sequence(&self) -> Option<u64> {
        self.journal.current_run_sequence()
    }

    /// Consumes this view and verifies one strictly extending successor journal.
    ///
    /// Equal, truncated, divergent, reordered, or old-object-changing reloads fail closed. The
    /// existing certified authority and semantic fold move forward without recertification or a
    /// full-prefix refold.
    pub fn verify_successor(self, successor: CommittedRunJournal) -> Result<Self> {
        let suffix_start = validate_strict_successor(&self.journal, &successor)?;
        let current_lifecycle =
            self.current_lifecycle
                .apply_suffix(&self.certified_spec, &successor, suffix_start)?;
        Ok(Self {
            journal: successor,
            certified_spec: self.certified_spec,
            current_lifecycle,
        })
    }

    pub(super) fn journal(&self) -> &CommittedRunJournal {
        &self.journal
    }

    pub(super) fn certified_spec(&self) -> &CertifiedTypedSpec {
        &self.certified_spec
    }

    pub(super) fn current_lifecycle_fold(&self) -> &CurrentLifecycleFold {
        self.current_lifecycle.as_ref()
    }
}

fn validate_strict_successor(
    current: &CommittedRunJournal,
    successor: &CommittedRunJournal,
) -> Result<usize> {
    if current.run_id != successor.run_id {
        return Err(invalid_successor(
            "successor journal belongs to a different run",
        ));
    }
    let prefix_len = current.records.len();
    if successor.records.len() <= prefix_len {
        return Err(invalid_successor(
            "successor journal must contain at least one new atomic commit",
        ));
    }
    if successor.records[..prefix_len] != current.records {
        return Err(invalid_successor(
            "successor journal does not preserve the exact committed prefix",
        ));
    }
    let expected_sequence = current.fold.current_run_sequence().checked_next()?;
    let suffix_first = &successor.records[prefix_len];
    if suffix_first.seq() != expected_sequence || suffix_first.ordinal().as_u32() != 0 {
        return Err(invalid_successor(
            "successor suffix does not begin at the checked next atomic commit",
        ));
    }
    for object in [&current.certified_spec.object, &current.certificate.object]
        .into_iter()
        .chain(current.objects.values())
    {
        let evidence_hash = object.evidence.evidence_hash()?;
        let Some(successor_object) = successor.object(&object.evidence.artifact_id, &evidence_hash)
        else {
            return Err(invalid_successor(
                "successor journal removed an existing retained object",
            ));
        };
        if successor_object.bytes != object.bytes || successor_object.evidence != object.evidence {
            return Err(invalid_successor(
                "successor journal changed an existing retained object",
            ));
        }
    }
    Ok(prefix_len)
}

fn invalid_successor(message: &str) -> StoreError {
    StoreError::PersistedEventMismatch {
        field: "journal_successor",
        message: message.to_owned(),
    }
}

/// Boxed future returned by retained artifact read providers.
pub type RetainedArtifactReadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<VerifiedRetainedArtifactBytes>> + Send + 'a>>;

/// Exact-object reader for explicit non-view fact, live-adapter, and cross-run boundaries.
///
/// Per-run history construction must use [`RunJournalStore::load_committed_journal`] instead.
pub trait RetainedArtifactReadProvider: Send + Sync {
    /// Reads artifact bytes and full evidence for one event-derived artifact requirement.
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a EventArtifactRequirement,
    ) -> RetainedArtifactReadFuture<'a>;
}

/// Verified retained bytes for an explicit non-view object read.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedRetainedArtifactBytes {
    bytes: Vec<u8>,
    evidence: ArtifactEvidenceRef,
}

impl fmt::Debug for VerifiedRetainedArtifactBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedRetainedArtifactBytes")
            .field("artifact_id", &self.evidence.artifact_id)
            .field("byte_len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl VerifiedRetainedArtifactBytes {
    /// Verifies bytes and evidence against one event-derived artifact requirement.
    pub fn new(
        bytes: Vec<u8>,
        evidence: ArtifactEvidenceRef,
        requirement: &EventArtifactRequirement,
    ) -> Result<Self> {
        validate_artifact_requirement_against_evidence(requirement, &evidence)?;
        verify_retained_artifact_bytes(&bytes, &evidence)?;
        Ok(Self { bytes, evidence })
    }

    /// Returns verified retained bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes this proof object into verified retained bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns full typed artifact evidence.
    pub fn evidence(&self) -> &ArtifactEvidenceRef {
        &self.evidence
    }
}

pub(super) struct JournalCommitRef<'a> {
    pub(super) events: &'a [KernelEventEnvelope],
}

pub(super) fn committed_journal_commits(
    records: &[KernelEventEnvelope],
) -> Vec<JournalCommitRef<'_>> {
    committed_journal_batches(records)
        .into_iter()
        .map(|batch| JournalCommitRef {
            events: &records[batch.record_range],
        })
        .collect()
}

fn committed_journal_batches(records: &[KernelEventEnvelope]) -> Vec<CommittedJournalBatch> {
    let mut batches = Vec::<CommittedJournalBatch>::new();
    for (position, record) in records.iter().enumerate() {
        if let Some(current) = batches.last_mut() {
            if current.seq == record.seq()
                && current.commit_key == *record.commit_key()
                && current.store_commit_order == record.store_commit_order()
            {
                current.record_range.end = position + 1;
                continue;
            }
        }
        batches.push(CommittedJournalBatch {
            seq: record.seq(),
            commit_key: record.commit_key().clone(),
            store_commit_order: record.store_commit_order(),
            record_range: position..position + 1,
        });
    }
    batches
}

pub(super) fn run_admission_root(records: &[KernelEventEnvelope]) -> Result<&events::RunAdmitted> {
    let Some(first) = records.first() else {
        return Err(StoreError::PersistedEventMismatch {
            field: "run_admission",
            message: "committed journal is empty".to_owned(),
        });
    };
    let KernelEventPayload::RunAdmitted(admission) = first.payload() else {
        return Err(StoreError::PersistedEventMismatch {
            field: "run_admission",
            message: "committed journal does not begin with RunAdmitted".to_owned(),
        });
    };
    Ok(admission)
}

fn verify_journal_objects(
    artifact_bytes: ArtifactByteAuthorityMap,
) -> Result<BTreeMap<ArtifactAuthorityKey, VerifiedJournalObject>> {
    let mut objects = BTreeMap::new();
    for (persisted_key, (bytes, evidence)) in artifact_bytes {
        let derived_key = artifact_authority_key(&evidence)?;
        if derived_key != persisted_key {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "artifact_authority_key",
            });
        }
        verify_retained_artifact_bytes(&bytes, &evidence)?;
        objects.insert(persisted_key, VerifiedJournalObject { bytes, evidence });
    }
    Ok(objects)
}

fn retain_loaded_required_authority(
    requirements: &[EventArtifactRequirement],
    loaded: ArtifactByteAuthorityMap,
) -> ArtifactByteAuthorityMap {
    let required_keys = requirements
        .iter()
        .map(|requirement| {
            (
                requirement.artifact_id.clone(),
                requirement.evidence_hash.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    loaded
        .into_iter()
        .filter(|(key, _)| required_keys.contains(key))
        .collect()
}

fn retain_required_journal_objects(
    requirements: &[EventArtifactRequirement],
    mut loaded: BTreeMap<ArtifactAuthorityKey, VerifiedJournalObject>,
) -> Result<BTreeMap<ArtifactAuthorityKey, VerifiedJournalObject>> {
    let mut retained = BTreeMap::<ArtifactAuthorityKey, VerifiedJournalObject>::new();
    for requirement in requirements {
        let key = (
            requirement.artifact_id.clone(),
            requirement.evidence_hash.clone(),
        );
        if let Some(object) = retained.get(&key) {
            validate_artifact_requirement_against_evidence(requirement, &object.evidence)?;
            continue;
        }
        let Some(object) = loaded.remove(&key) else {
            return Err(StoreError::MissingArtifact {
                artifact_id: requirement.artifact_id.clone(),
            });
        };
        validate_artifact_requirement_against_evidence(requirement, &object.evidence)?;
        retained.insert(key, object);
    }
    Ok(retained)
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn required_artifact_byte_authority(
    records: &[KernelEventEnvelope],
    authority: &ArtifactByteAuthorityMap,
) -> ArtifactByteAuthorityMap {
    records
        .iter()
        .flat_map(|record| event_artifact_requirements(record.payload()))
        .filter_map(|requirement| {
            let key = (requirement.artifact_id, requirement.evidence_hash);
            authority.get(&key).cloned().map(|object| (key, object))
        })
        .collect()
}

fn take_bootstrap_object(
    objects: &mut BTreeMap<ArtifactAuthorityKey, VerifiedJournalObject>,
    artifact: &events::RunArtifactEvidenceRef,
    expected_role: ArtifactRole,
) -> Result<BootstrapObject> {
    if artifact.role != expected_role {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: artifact.artifact_id.clone(),
            field: "artifact_role",
        });
    }
    let key = (artifact.artifact_id.clone(), artifact.evidence_hash.clone());
    let object = objects
        .remove(&key)
        .ok_or_else(|| StoreError::MissingArtifact {
            artifact_id: artifact.artifact_id.clone(),
        })?;
    let schema_id =
        artifact
            .schema_id
            .clone()
            .ok_or_else(|| StoreError::ArtifactEvidenceMismatch {
                artifact_id: artifact.artifact_id.clone(),
                field: "schema_id",
            })?;
    let content_digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        sha256_digest_bytes(&object.bytes),
    );
    let content_ref = ContentRef::new(schema_id, content_digest)?;
    Ok(BootstrapObject {
        content_ref,
        object,
    })
}

pub(super) fn validate_journal_record_order(events: &[KernelEventEnvelope]) -> Result<()> {
    let mut stream_run_id: Option<RunId> = None;
    let mut current_seq: Option<StreamSeq> = None;
    let mut current_commit_key: Option<CommitKey> = None;
    let mut current_store_commit_order: Option<StoreCommitOrder> = None;
    let mut committed_keys = BTreeSet::<CommitKey>::new();
    let mut expected_ordinal = 0_u32;

    for event in events {
        match &stream_run_id {
            Some(run_id) if event.run_id() != run_id => {
                return Err(StoreError::PersistedEventMismatch {
                    field: "run_id",
                    message: "persisted run stream contains events for multiple runs".to_owned(),
                });
            }
            Some(_) => {}
            None => stream_run_id = Some(event.run_id().clone()),
        }

        match current_seq {
            Some(seq) if event.seq() == seq => {
                if current_commit_key.as_ref() != Some(event.commit_key()) {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "commit_key",
                        message: format!(
                            "persisted run stream seq {} contains multiple commit keys",
                            event.seq()
                        ),
                    });
                }
                if current_store_commit_order != Some(event.store_commit_order()) {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "store_commit_order",
                        message: format!(
                            "persisted run stream seq {} contains multiple store append coordinates",
                            event.seq()
                        ),
                    });
                }
            }
            Some(seq) => {
                let expected_next = seq.checked_next()?;
                if event.seq() != expected_next {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "seq",
                        message: format!(
                            "persisted run stream expected seq {expected_next} but found {}",
                            event.seq()
                        ),
                    });
                }
                let previous_store_commit_order = current_store_commit_order.ok_or_else(|| {
                    StoreError::PersistedEventMismatch {
                        field: "store_commit_order",
                        message: "persisted run stream has no coordinate for the previous commit"
                            .to_owned(),
                    }
                })?;
                if event.store_commit_order() <= previous_store_commit_order {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "store_commit_order",
                        message: format!(
                            "persisted run stream store append coordinate {} is not greater than the previous commit",
                            event.store_commit_order().as_u64()
                        ),
                    });
                }
                if !committed_keys.insert(event.commit_key().clone()) {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "commit_key",
                        message: "persisted run journal reuses a commit key at a later sequence"
                            .to_owned(),
                    });
                }
                current_seq = Some(expected_next);
                current_commit_key = Some(event.commit_key().clone());
                current_store_commit_order = Some(event.store_commit_order());
                expected_ordinal = 0;
            }
            None => {
                if event.seq() != StreamSeq::FIRST {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "seq",
                        message: format!(
                            "persisted run stream expected first seq {} but found {}",
                            StreamSeq::FIRST,
                            event.seq()
                        ),
                    });
                }
                if event.store_commit_order() < StoreCommitOrder::FIRST {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "store_commit_order",
                        message: "persisted run stream first commit has a non-positive store append coordinate"
                            .to_owned(),
                    });
                }
                committed_keys.insert(event.commit_key().clone());
                current_seq = Some(StreamSeq::FIRST);
                current_commit_key = Some(event.commit_key().clone());
                current_store_commit_order = Some(event.store_commit_order());
            }
        }

        if event.ordinal().as_u32() != expected_ordinal {
            return Err(StoreError::PersistedEventMismatch {
                field: "ordinal",
                message: format!(
                    "persisted run stream expected ordinal {expected_ordinal} for seq {} but found {}",
                    event.seq(),
                    event.ordinal()
                ),
            });
        }
        expected_ordinal = expected_ordinal
            .checked_add(1)
            .ok_or(StoreError::SequenceOverflow)?;
    }

    Ok(())
}

#[cfg(test)]
mod journal_order_tests {
    use super::*;

    fn event(run_id: &RunId, seq: u64, store_commit_order: u64) -> KernelEventEnvelope {
        test_support::persisted_kernel_event_envelope_for_test(
            run_id,
            seq,
            store_commit_order,
            CommitKey::new(format!("order-{seq}")).expect("commit key"),
            KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: test_support::fixed_spec_hash_for_test(1),
                node_id: test_support::fixed_node_id_for_test(2),
                attempt_id: test_support::fixed_attempt_id_for_test(3),
                attempt_no: 1,
                state_kind: test_support::fixed_state_kind_for_test(4),
                state_version: StateVersion::new("mfm.test.state.v1").expect("state version"),
            }),
        )
    }

    #[test]
    fn committed_journal_coordinates_are_positive_and_strictly_increasing() {
        let run_id = RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            test_support::fixed_digest_bytes_for_test(5),
        );

        let first = event(&run_id, 1, 2);
        let repeated = event(&run_id, 2, 2);
        let error = validate_journal_record_order(&[first, repeated])
            .expect_err("repeated store coordinate must reject");
        assert!(error.to_string().contains("not greater"), "{error}");

        let non_positive = event(&run_id, 1, 0);
        let error = validate_journal_record_order(&[non_positive])
            .expect_err("non-positive first store coordinate must reject");
        assert!(error.to_string().contains("non-positive"), "{error}");
    }
}

#[cfg(test)]
mod retained_artifact_tests {
    use super::*;

    fn digest(bytes: &[u8]) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
    }

    fn artifact_evidence(bytes: &[u8]) -> ArtifactEvidenceRef {
        let digest = digest(bytes);
        ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: bytes.len() as u64,
            media_type: MediaType::new("application/json").expect("media type"),
            schema_id: Some(
                SchemaId::new(
                    "mfm.test.retained",
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    sha256_digest_bytes(b"mfm.test.retained"),
                )
                .expect("schema id"),
            ),
            semantic_type_id: Some(
                SemanticTypeId::new(
                    "mfm.test",
                    "retained",
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    sha256_digest_bytes(b"mfm.test:retained"),
                )
                .expect("semantic id"),
            ),
            producer_node_id: Some(NodeId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"retained-producer"),
            )),
            producer_seed_id: None,
            artifact_role: ArtifactRole::StateOutput,
        }
    }

    fn exact_requirement(evidence: &ArtifactEvidenceRef) -> EventArtifactRequirement {
        EventArtifactRequirement {
            source: EventArtifactReferenceSource::StateOutput,
            artifact_id: evidence.artifact_id.clone(),
            evidence_hash: evidence.evidence_hash().expect("evidence hash"),
            digest: Some(evidence.digest.clone()),
            byte_len: Some(evidence.byte_len),
            media_type: Some(evidence.media_type.clone()),
            schema_id: evidence.schema_id.clone(),
            semantic_type_id: evidence.semantic_type_id.clone(),
            producer_node_id: evidence.producer_node_id.clone(),
            producer_seed_id: None,
            artifact_role: Some(evidence.artifact_role),
        }
    }

    #[test]
    fn verified_retained_bytes_and_prepared_bytes_share_exact_content_authority() {
        let bytes = br#"{"retained":true}"#.to_vec();
        let evidence = artifact_evidence(&bytes);
        let requirement = exact_requirement(&evidence);

        let verified =
            VerifiedRetainedArtifactBytes::new(bytes.clone(), evidence.clone(), &requirement)
                .expect("verified retained bytes");
        assert_eq!(verified.bytes(), bytes);
        assert_eq!(verified.evidence(), &evidence);
        PreparedArtifactBytes::new(bytes.clone(), evidence.clone()).expect("prepared bytes");

        for (name, candidate_bytes, candidate_evidence) in [
            (
                "tampered bytes",
                br#"{"retained":false}"#.to_vec(),
                evidence.clone(),
            ),
            ("wrong digest", bytes.clone(), {
                let mut changed = evidence.clone();
                changed.digest = digest(b"wrong digest");
                changed
            }),
            ("wrong length", bytes.clone(), {
                let mut changed = evidence.clone();
                changed.byte_len += 1;
                changed
            }),
        ] {
            let candidate_requirement = exact_requirement(&candidate_evidence);
            let error = VerifiedRetainedArtifactBytes::new(
                candidate_bytes,
                candidate_evidence,
                &candidate_requirement,
            )
            .expect_err(name);
            assert!(
                matches!(error, StoreError::ArtifactEvidenceMismatch { .. }),
                "{name}: {error:?}"
            );
        }
    }

    #[test]
    fn retained_requirement_rejects_every_metadata_and_binding_mismatch() {
        let bytes = br#"{"retained":true}"#;
        let evidence = artifact_evidence(bytes);
        let base = exact_requirement(&evidence);
        let other_digest = digest(b"other");
        let other_artifact =
            ArtifactId::from_digest(other_digest.algorithm(), *other_digest.digest());
        let other_schema = SchemaId::new(
            "mfm.test.other",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.other"),
        )
        .expect("other schema");
        let other_semantic = SemanticTypeId::new(
            "mfm.test",
            "other",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test:other"),
        )
        .expect("other semantic");
        let other_node = NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"other-node"),
        );
        let other_seed = SeedId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"other-seed"),
        );

        let mut mismatches = Vec::new();
        let mut requirement = base.clone();
        requirement.artifact_id = other_artifact;
        mismatches.push(("artifact_id", requirement));
        let mut requirement = base.clone();
        requirement.evidence_hash = other_digest.clone();
        mismatches.push(("evidence_hash", requirement));
        let mut requirement = base.clone();
        requirement.digest = Some(other_digest);
        mismatches.push(("digest", requirement));
        let mut requirement = base.clone();
        requirement.byte_len = Some(evidence.byte_len + 1);
        mismatches.push(("byte_len", requirement));
        let mut requirement = base.clone();
        requirement.media_type = Some(MediaType::new("application/octet-stream").expect("media"));
        mismatches.push(("media_type", requirement));
        let mut requirement = base.clone();
        requirement.schema_id = Some(other_schema);
        mismatches.push(("schema_id", requirement));
        let mut requirement = base.clone();
        requirement.semantic_type_id = Some(other_semantic);
        mismatches.push(("semantic_type_id", requirement));
        let mut requirement = base.clone();
        requirement.producer_node_id = Some(other_node);
        mismatches.push(("producer_node_id", requirement));
        let mut requirement = base.clone();
        requirement.producer_seed_id = Some(other_seed);
        mismatches.push(("producer_seed_id", requirement));
        let mut requirement = base;
        requirement.artifact_role = Some(ArtifactRole::TypedConfig);
        mismatches.push(("artifact_role", requirement));

        for (name, requirement) in mismatches {
            let error = validate_artifact_requirement_against_evidence(&requirement, &evidence)
                .expect_err(name);
            assert!(
                matches!(error, StoreError::ArtifactEvidenceMismatch { .. }),
                "{name}: {error:?}"
            );
        }
    }
}
