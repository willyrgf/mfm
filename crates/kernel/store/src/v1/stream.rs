use super::*;

/// One atomically committed run-stream batch reconstructed from persisted envelopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedRunStreamCommit {
    seq: StreamSeq,
    commit_key: CommitKey,
    pub(super) events: Vec<KernelEventEnvelope>,
}

impl CommittedRunStreamCommit {
    /// Returns the stream sequence shared by this committed batch.
    pub fn seq(&self) -> StreamSeq {
        self.seq
    }

    /// Returns the commit key shared by this committed batch.
    pub fn commit_key(&self) -> &CommitKey {
        &self.commit_key
    }

    /// Returns the events committed atomically in ordinal order.
    pub fn events(&self) -> &[KernelEventEnvelope] {
        &self.events
    }
}

/// Store-owned verified run-stream authority.
///
/// This type proves store-level ordering, commit grouping, projection rebuild, next sequence,
/// and artifact role requirements for one run stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedRunStream {
    run_id: RunId,
    events: Vec<KernelEventEnvelope>,
    commits: Vec<CommittedRunStreamCommit>,
    projection: ProjectionSnapshot,
    next_seq: StreamSeq,
    artifact_requirements: Vec<EventArtifactRequirement>,
    artifact_bytes: ArtifactByteAuthorityMap,
}

impl CommittedRunStream {
    /// Rebuilds store-owned stream authority from persisted event envelopes.
    pub fn from_events(run_id: RunId, events: Vec<KernelEventEnvelope>) -> Result<Self> {
        Self::from_events_with_artifact_bytes(run_id, events, &ArtifactByteAuthorityMap::new())
    }

    /// Rebuilds store-owned stream authority from persisted event envelopes and retained bytes.
    pub fn from_events_with_artifact_bytes(
        run_id: RunId,
        events: Vec<KernelEventEnvelope>,
        artifact_bytes: &ArtifactByteAuthorityMap,
    ) -> Result<Self> {
        if let Some(event) = events.iter().find(|event| event.run_id() != &run_id) {
            return Err(StoreError::PersistedEventMismatch {
                field: "run_id",
                message: format!(
                    "run stream requested run {} but stream contains {}",
                    run_id,
                    event.run_id()
                ),
            });
        }
        ProjectionSnapshot::validate_run_stream(&events)?;
        let commits = committed_run_stream_commits(&events);
        let projection = ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
            &events,
            artifact_bytes,
        )?;
        let next_seq = next_seq_after_committed_stream(&events)?;
        let artifact_requirements = events
            .iter()
            .flat_map(|event| event.payload().artifact_requirements())
            .collect();
        Ok(Self {
            run_id,
            events,
            commits,
            projection,
            next_seq,
            artifact_requirements,
            artifact_bytes: artifact_bytes.clone(),
        })
    }

    /// Returns the run id covered by this stream.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the committed envelopes in stream order.
    pub fn events(&self) -> &[KernelEventEnvelope] {
        &self.events
    }

    /// Returns atomically committed batches reconstructed from sequence and commit key.
    pub fn commits(&self) -> &[CommittedRunStreamCommit] {
        &self.commits
    }

    /// Returns the projection rebuilt from this verified stream.
    pub fn projection(&self) -> &ProjectionSnapshot {
        &self.projection
    }

    /// Returns the next store-owned stream sequence for this run.
    pub fn next_seq(&self) -> StreamSeq {
        self.next_seq
    }

    /// Returns artifact role requirements referenced by this stream.
    pub fn artifact_requirements(&self) -> &[EventArtifactRequirement] {
        &self.artifact_requirements
    }

    /// Returns exact retained artifact-byte authority used to rebuild this stream projection.
    pub fn artifact_byte_authority(&self) -> &ArtifactByteAuthorityMap {
        &self.artifact_bytes
    }
}

/// Returns canonical JSON bytes for a committed run stream.
///
/// The encoded shape contains store envelope fields plus canonical typed payload JSON. Decoding it
/// with [`committed_run_stream_from_canonical_json_slice`] re-derives event ids, payload hashes,
/// logical keys, projection state, and commit grouping through the normal store authority path.
pub fn committed_run_stream_canonical_json(
    committed: &CommittedRunStream,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "stream_version": 1,
        "run_id": committed.run_id().as_str(),
        "events": committed.events().iter().map(kernel_event_envelope_json).collect::<Vec<_>>(),
    }))
}

/// Rebuilds store-owned stream authority from canonical committed-stream JSON.
///
/// `artifact_bytes` must contain exact byte authority for artifact-bearing events such as terminal
/// state-output cells. The decoder fails closed when the embedded run id differs from
/// `expected_run_id` or when any envelope field no longer derives from its typed payload.
pub fn committed_run_stream_from_canonical_json_slice(
    expected_run_id: &RunId,
    bytes: &[u8],
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<CommittedRunStream> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let json: serde_json::Value = serde_json::from_slice(canonical.as_bytes())
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    if required_u64(&json, "stream_version")? != 1 {
        return Err(StoreError::Identity(
            "unsupported committed stream version".to_owned(),
        ));
    }
    let run_id: RunId = parse_identity(required_str(&json, "run_id")?)?;
    if &run_id != expected_run_id {
        return Err(StoreError::PersistedEventMismatch {
            field: "run_id",
            message: format!(
                "committed stream artifact covers run {} but import expected {}",
                run_id, expected_run_id
            ),
        });
    }
    let events = parse_vec(&json, "events", parse_kernel_event_envelope)?;
    CommittedRunStream::from_events_with_artifact_bytes(run_id, events, artifact_bytes)
}

/// Boxed future returned by retained artifact read providers.
pub type RetainedArtifactReadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<VerifiedRunArtifactBytes>> + Send + 'a>>;

/// Store-owned retained artifact reader for verified run-history construction.
pub trait RetainedArtifactReadProvider: Send + Sync {
    /// Reads artifact bytes and full evidence for one event-derived artifact requirement.
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a EventArtifactRequirement,
    ) -> RetainedArtifactReadFuture<'a>;
}

/// Verified retained artifact bytes and full typed evidence for one run-history artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRunArtifactBytes {
    bytes: Vec<u8>,
    evidence: ArtifactEvidenceRef,
}

impl VerifiedRunArtifactBytes {
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

/// Verified retained evidence for every artifact required by one committed run stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRunArtifactStore {
    run_id: RunId,
    requirements: Vec<EventArtifactRequirement>,
    artifacts: BTreeMap<ArtifactAuthorityKey, VerifiedRunArtifactBytes>,
}

impl VerifiedRunArtifactStore {
    /// Loads and verifies all retained artifacts required by a committed run stream.
    pub async fn from_committed_stream<P>(
        committed: &CommittedRunStream,
        provider: &P,
    ) -> Result<Self>
    where
        P: RetainedArtifactReadProvider + ?Sized,
    {
        let mut artifacts = BTreeMap::new();
        for requirement in committed.artifact_requirements() {
            let artifact = provider.read_retained_artifact(requirement).await?;
            validate_artifact_requirement_against_evidence(requirement, artifact.evidence())?;
            insert_verified_run_artifact(&mut artifacts, artifact)?;
        }
        Ok(Self {
            run_id: committed.run_id().clone(),
            requirements: committed.artifact_requirements().to_vec(),
            artifacts,
        })
    }

    /// Run id covered by this verified artifact store.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns true when the committed stream required no retained artifacts.
    pub fn is_empty(&self) -> bool {
        self.requirements.is_empty()
    }

    /// Returns verified retained artifact bytes matching an event-derived requirement.
    pub fn artifact_for_requirement(
        &self,
        requirement: &EventArtifactRequirement,
    ) -> Option<&VerifiedRunArtifactBytes> {
        let key = (
            requirement.artifact_id.clone(),
            requirement.evidence_hash.clone(),
        );
        let artifact = self.artifacts.get(&key)?;
        validate_artifact_requirement_against_evidence(requirement, artifact.evidence())
            .ok()
            .map(|()| artifact)
    }

    /// Iterates verified retained artifacts by exact artifact authority key.
    pub fn artifacts(
        &self,
    ) -> impl Iterator<Item = (&ArtifactAuthorityKey, &VerifiedRunArtifactBytes)> {
        self.artifacts.iter()
    }

    /// Exports exact retained artifact-byte authority for projection rebuilds.
    pub fn artifact_byte_authority_map(&self) -> ArtifactByteAuthorityMap {
        self.artifacts
            .iter()
            .map(|(key, artifact)| {
                (
                    key.clone(),
                    (artifact.bytes().to_vec(), artifact.evidence().clone()),
                )
            })
            .collect()
    }

    /// Verifies this retained artifact store covers a committed run stream exactly.
    pub fn validate_committed_stream(&self, committed: &CommittedRunStream) -> Result<()> {
        if self.run_id != *committed.run_id() {
            return Err(StoreError::PersistedEventMismatch {
                field: "run_id",
                message: format!(
                    "retained artifact store for {} cannot verify run {}",
                    self.run_id,
                    committed.run_id()
                ),
            });
        }
        if self.requirements != committed.artifact_requirements() {
            return Err(StoreError::ProjectionConflict {
                key: "retained_artifacts:requirements".to_owned(),
                message: "retained artifact store requirements do not match committed run stream"
                    .to_owned(),
            });
        }
        self.validate_requirements(committed.artifact_requirements())
    }

    fn validate_requirements(&self, requirements: &[EventArtifactRequirement]) -> Result<()> {
        for requirement in requirements {
            let key = (
                requirement.artifact_id.clone(),
                requirement.evidence_hash.clone(),
            );
            let Some(artifact) = self.artifacts.get(&key) else {
                return Err(StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            };
            validate_artifact_requirement_against_evidence(requirement, artifact.evidence())?;
        }
        Ok(())
    }
}

fn insert_verified_run_artifact(
    artifacts: &mut BTreeMap<ArtifactAuthorityKey, VerifiedRunArtifactBytes>,
    artifact: VerifiedRunArtifactBytes,
) -> Result<()> {
    let key = artifact_authority_key(artifact.evidence())?;
    if let Some(existing) = artifacts.get(&key) {
        if existing != &artifact {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: artifact.evidence().artifact_id.clone(),
                field: "retained_artifact",
            });
        }
        return Ok(());
    }
    artifacts.insert(key, artifact);
    Ok(())
}

pub(super) fn committed_run_stream_commits(
    events: &[KernelEventEnvelope],
) -> Vec<CommittedRunStreamCommit> {
    let mut commits = Vec::<CommittedRunStreamCommit>::new();
    for event in events {
        if let Some(current) = commits.last_mut() {
            if current.seq == event.seq() && &current.commit_key == event.commit_key() {
                current.events.push(event.clone());
                continue;
            }
        }
        commits.push(CommittedRunStreamCommit {
            seq: event.seq(),
            commit_key: event.commit_key().clone(),
            events: vec![event.clone()],
        });
    }
    commits
}

fn next_seq_after_committed_stream(events: &[KernelEventEnvelope]) -> Result<StreamSeq> {
    let Some(event) = events.last() else {
        return Ok(StreamSeq::FIRST);
    };
    event.seq().checked_next()
}

pub(super) fn validate_run_stream_order(events: &[KernelEventEnvelope]) -> Result<()> {
    let mut stream_run_id: Option<RunId> = None;
    let mut current_seq: Option<StreamSeq> = None;
    let mut current_commit_key: Option<CommitKey> = None;
    let mut current_store_commit_order: Option<StoreCommitOrder> = None;
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
mod stream_order_tests {
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
    fn committed_stream_coordinates_are_positive_and_strictly_increasing() {
        let run_id = RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            test_support::fixed_digest_bytes_for_test(5),
        );

        let first = event(&run_id, 1, 2);
        let repeated = event(&run_id, 2, 2);
        let error = validate_run_stream_order(&[first, repeated])
            .expect_err("repeated store coordinate must reject");
        assert!(error.to_string().contains("not greater"), "{error}");

        let non_positive = event(&run_id, 1, 0);
        let error = validate_run_stream_order(&[non_positive])
            .expect_err("non-positive first store coordinate must reject");
        assert!(error.to_string().contains("non-positive"), "{error}");
    }
}
