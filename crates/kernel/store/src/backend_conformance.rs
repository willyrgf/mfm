//! Shared mechanical backend contract checks for Memory and durable adapters.

use std::sync::Arc;

use mfm_canonical::raw_content_digest;
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, RunId, SchemaId,
    SequentialControlAddress, StableId,
};
use mfm_journal::single_trust::{
    BindingDescriptor, ConfigurationHeadProjection, ImmutableObject, PreparationMode,
    PreparationRef, RunAdmitted, RunFrame, RunRecord, StateConcluded, StateOutcome, StatePrepared,
    ValueRef,
};

use crate::backend::{
    BackendAppendCommand, BackendAppendOutcome, BackendConfigurationOutcome, BackendError,
    ConfigurationAppendCommand, RawConfigurationRevision, RawFactPublication, RawFactSnapshot,
    RawFrameBytes, RawHistoryLoadLimit, RawRunPrefix, StructuredStoreBackend,
    StructuredStoreIdentity,
};

/// One transaction boundary exercised by the PostgreSQL fault probes.
#[derive(Clone, Copy)]
pub enum AtomicRollbackProbe {
    /// A history frame/head transaction.
    History,
    /// A history frame plus fact publication transaction.
    FactPublication,
    /// A configuration revision/head transaction.
    Configuration,
}

/// One independent same-epoch process CAS stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessRaceStage {
    /// Two processes compete for one genesis frame.
    Admission,
    /// Two processes compete for the first selected preparation.
    Preparation,
    /// Two processes compete to replace one selected preparation.
    Replacement,
    /// Two processes compete to conclude one selected preparation.
    Conclusion,
}

impl ProcessRaceStage {
    /// Every process race stage in the order used by the PostgreSQL harness.
    pub const ALL: [Self; 4] = [
        Self::Admission,
        Self::Preparation,
        Self::Replacement,
        Self::Conclusion,
    ];

    /// Returns the stable environment value used by the child test process.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admission => "admission",
            Self::Preparation => "preparation",
            Self::Replacement => "replacement",
            Self::Conclusion => "conclusion",
        }
    }

    /// Parses the stable child-process environment value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "admission" => Some(Self::Admission),
            "preparation" => Some(Self::Preparation),
            "replacement" => Some(Self::Replacement),
            "conclusion" => Some(Self::Conclusion),
            _ => None,
        }
    }

    /// Returns the number of frames seeded before the competing append.
    pub const fn prefix_len(self) -> usize {
        match self {
            Self::Admission => 0,
            Self::Preparation => 1,
            Self::Replacement | Self::Conclusion => 2,
        }
    }
}

/// Exercises one append transaction with an externally installed SQL failpoint.
pub async fn exercise_atomic_rollback(
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
    probe: AtomicRollbackProbe,
) -> Result<(), BackendError> {
    let run_id = RunId::parse(match probe {
        AtomicRollbackProbe::History => {
            "run:sha256-jcs-v1:7123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
        AtomicRollbackProbe::FactPublication => {
            "run:sha256-jcs-v1:8123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
        AtomicRollbackProbe::Configuration => {
            "run:sha256-jcs-v1:9123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
    })
    .map_err(|_| BackendError::Storage)?;
    let first_bytes = br#"{"kind":"rollback-first"}"#;
    let first_digest = raw_content_digest(first_bytes);
    let first_head = ContentDigest::parse(
        "content:sha256-v1:1111111111111111111111111111111111111111111111111111111111111111",
    )
    .map_err(|_| BackendError::Storage)?;
    let first_request = AppendRequestId::new("rollback-first-0123456789abcdef")
        .map_err(|_| BackendError::Storage)?;
    let first = BackendAppendCommand::new(
        &identity,
        &run_id,
        1,
        &first_request,
        first_bytes,
        &first_digest,
        &first_head,
        None,
        true,
        None,
    );

    match probe {
        AtomicRollbackProbe::History => {
            if backend.compare_and_append(&first).await.is_ok() {
                return Err(BackendError::Conflict);
            }
            let prefix = backend
                .load_complete_prefix(&run_id, RawHistoryLoadLimit::new(4, 4096))
                .await?;
            if prefix.is_some() {
                return Err(BackendError::Conflict);
            }
        }
        AtomicRollbackProbe::FactPublication => {
            if !matches!(
                backend.compare_and_append(&first).await?,
                BackendAppendOutcome::NewlyCommitted | BackendAppendOutcome::Found(_)
            ) {
                return Err(BackendError::Conflict);
            }
            let proposal_schema = SchemaId::new(
                "mfm.test.rollback-proposals",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([1; 32]),
            )
            .map_err(|_| BackendError::Storage)?;
            let proposal_set_ref =
                ContentRef::new(proposal_schema, raw_content_digest(b"proposals"))
                    .map_err(|_| BackendError::Storage)?;
            let publication = RawFactPublication::new(1, run_id.clone(), 2, proposal_set_ref)?;
            let second_bytes = br#"{"kind":"rollback-second"}"#;
            let second_digest = raw_content_digest(second_bytes);
            let second_head = ContentDigest::parse(
                "content:sha256-v1:2222222222222222222222222222222222222222222222222222222222222222",
            )
            .map_err(|_| BackendError::Storage)?;
            let second_request = AppendRequestId::new("rollback-second-0123456789abcdef")
                .map_err(|_| BackendError::Storage)?;
            let second = BackendAppendCommand::new(
                &identity,
                &run_id,
                2,
                &second_request,
                second_bytes,
                &second_digest,
                &second_head,
                Some(&first_head),
                false,
                Some(&publication),
            );
            if backend.compare_and_append(&second).await.is_ok() {
                return Err(BackendError::Conflict);
            }
            let prefix = backend
                .load_complete_prefix(&run_id, RawHistoryLoadLimit::new(4, 4096))
                .await?
                .ok_or(BackendError::Storage)?;
            let fact_head = backend.load_facts().await?.head_sequence();
            if prefix.frames().len() != 1 || fact_head != 0 {
                return Err(BackendError::Conflict);
            }
        }
        AtomicRollbackProbe::Configuration => {
            let before = backend.load_configuration().await?;
            let bytes = br#"{"kind":"rollback-configuration"}"#;
            let schema = SchemaId::new(
                "mfm.test.rollback-configuration",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([2; 32]),
            )
            .map_err(|_| BackendError::Storage)?;
            let content_ref = ContentRef::new(schema, raw_content_digest(bytes))
                .map_err(|_| BackendError::Storage)?;
            let request = AppendRequestId::new("rollback-configuration-0123456789")
                .map_err(|_| BackendError::Storage)?;
            let command = ConfigurationAppendCommand::new(
                &identity,
                u64::try_from(before.len()).map_err(|_| BackendError::Capacity)?,
                &request,
                bytes,
                &content_ref,
            );
            if backend
                .compare_and_append_configuration(&command)
                .await
                .is_ok()
            {
                return Err(BackendError::Conflict);
            }
            if backend.load_configuration().await? != before {
                return Err(BackendError::Conflict);
            }
        }
    }
    Ok(())
}

fn restart_probe_value(seed: u8, label: &str) -> Result<(ValueRef, ImmutableObject), BackendError> {
    let schema = SchemaId::new(
        "mfm.test.primary-restart",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([seed; 32]),
    )
    .map_err(|_| BackendError::Storage)?;
    let contract = ContentRef::new(
        schema.clone(),
        raw_content_digest(format!("{label}-contract").as_bytes()),
    )
    .map_err(|_| BackendError::Storage)?;
    let canonical_json = format!(r#"{{"value":"{label}"}}"#);
    let value_ref = ContentRef::new(schema, raw_content_digest(canonical_json.as_bytes()))
        .map_err(|_| BackendError::Storage)?;
    let object = ImmutableObject::new(
        StableId::new("mfm.value").map_err(|_| BackendError::Storage)?,
        value_ref.clone(),
        canonical_json,
    )
    .map_err(|_| BackendError::Storage)?;
    Ok((ValueRef::new(contract, value_ref), object))
}

fn restart_probe_content(seed: u8, label: &str) -> Result<ContentRef, BackendError> {
    let schema = SchemaId::new(
        "mfm.test.primary-restart-reference",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([seed; 32]),
    )
    .map_err(|_| BackendError::Storage)?;
    ContentRef::new(schema, raw_content_digest(label.as_bytes())).map_err(|_| BackendError::Storage)
}

async fn append_restart_probe_frame(
    backend: &Arc<dyn StructuredStoreBackend>,
    identity: &StructuredStoreIdentity,
    frame: &RunFrame,
    previous_head: Option<&ContentDigest>,
) -> Result<ContentDigest, BackendError> {
    let bytes = frame.canonical_bytes().map_err(|_| BackendError::Storage)?;
    let frame_digest = raw_content_digest(bytes.as_bytes());
    let head_digest = frame
        .head_digest(previous_head)
        .map_err(|_| BackendError::Storage)?;
    let command = BackendAppendCommand::new(
        identity,
        frame.run_id(),
        frame.expected_sequence(),
        frame.append_request_id(),
        bytes.as_bytes(),
        &frame_digest,
        &head_digest,
        previous_head,
        frame.record().is_admission(),
        None,
    );
    if !matches!(
        backend.compare_and_append(&command).await?,
        BackendAppendOutcome::NewlyCommitted
    ) {
        return Err(BackendError::Conflict);
    }
    Ok(head_digest)
}

/// Appends an admitted prefix ending in a real `StatePrepared` frame for restart verification.
pub async fn append_primary_restart_probe(
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
) -> Result<RunId, BackendError> {
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:a123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .map_err(|_| BackendError::Storage)?;
    let (input, input_object) = restart_probe_value(31, "restart-input")?;
    let (intent, intent_object) = restart_probe_value(32, "restart-intent")?;
    let program_ref = restart_probe_content(33, "restart-program")?;
    let configuration_ref = restart_probe_content(34, "restart-configuration")?;
    let physical_target_ref = restart_probe_content(35, "restart-target")?;
    let state_ref = restart_probe_content(36, "restart-state")?;
    let capability_ref = restart_probe_content(37, "restart-capability")?;
    let adapter_ref = restart_probe_content(38, "restart-adapter")?;
    let binding = BindingDescriptor::new(
        state_ref,
        Some(capability_ref),
        Some(adapter_ref),
        physical_target_ref,
        None,
        None,
    )
    .map_err(|_| BackendError::Storage)?;
    let binding_ref = binding.content_ref().map_err(|_| BackendError::Storage)?;
    let admission = RunAdmitted::new(
        identity.scope().clone(),
        identity.epoch(),
        run_id.clone(),
        identity.tenant().clone(),
        StableId::new("mfm.test.primary-restart@1").map_err(|_| BackendError::Storage)?,
        program_ref,
        input.clone(),
        ConfigurationHeadProjection::new(1, configuration_ref)
            .map_err(|_| BackendError::Storage)?,
        Vec::new(),
    )
    .map_err(|_| BackendError::Storage)?;
    let admission_frame = RunFrame::new(
        run_id.clone(),
        identity.scope().clone(),
        identity.epoch(),
        1,
        AppendRequestId::new("primary-restart-admission-0123456789")
            .map_err(|_| BackendError::Storage)?,
        RunRecord::RunAdmitted(admission),
        vec![input_object.clone()],
    )
    .map_err(|_| BackendError::Storage)?;
    let first_head =
        append_restart_probe_frame(&backend, &identity, &admission_frame, None).await?;
    let prepared = StatePrepared::new(
        SequentialControlAddress::new(0, Vec::new()).map_err(|_| BackendError::Storage)?,
        0,
        input.clone(),
        intent.clone(),
        None,
        None,
        PreparationMode::Read {
            total_attempt_bound: 1,
        },
        binding,
        binding_ref,
        None,
        4096,
    )
    .map_err(|_| BackendError::Storage)?;
    let prepared_frame = RunFrame::new(
        run_id.clone(),
        identity.scope().clone(),
        identity.epoch(),
        2,
        AppendRequestId::new("primary-restart-prepared-0123456789")
            .map_err(|_| BackendError::Storage)?,
        RunRecord::StatePrepared(prepared),
        vec![intent_object.clone()],
    )
    .map_err(|_| BackendError::Storage)?;
    let prepared_head =
        append_restart_probe_frame(&backend, &identity, &prepared_frame, Some(&first_head)).await?;
    let conclusion = StateConcluded::Access {
        occurrence: SequentialControlAddress::new(0, Vec::new())
            .map_err(|_| BackendError::Storage)?,
        preparation: PreparationRef::new(run_id.clone(), 2, 1),
        evidence: intent.clone(),
        outcome: StateOutcome::Success(input),
        fact_proposals: None,
        fact_selection: None,
        fact_publication: None,
    };
    let conclusion_frame = RunFrame::new(
        run_id.clone(),
        identity.scope().clone(),
        identity.epoch(),
        3,
        AppendRequestId::new("primary-restart-conclusion-0123456789")
            .map_err(|_| BackendError::Storage)?,
        RunRecord::StateConcluded(conclusion),
        vec![intent_object, input_object],
    )
    .map_err(|_| BackendError::Storage)?;
    append_restart_probe_frame(&backend, &identity, &conclusion_frame, Some(&prepared_head))
        .await?;
    Ok(run_id)
}

/// Verifies that the restart probe retains preparation and conclusion after primary recovery.
pub async fn verify_primary_restart_probe(
    backend: Arc<dyn StructuredStoreBackend>,
    run_id: RunId,
) -> Result<(), BackendError> {
    let prefix = backend
        .load_complete_prefix(&run_id, RawHistoryLoadLimit::new(4, 16 * 1024))
        .await?
        .ok_or(BackendError::Storage)?;
    if prefix.frames().len() != 3 {
        return Err(BackendError::Conflict);
    }
    let first: RunFrame = serde_json::from_slice(prefix.frames()[0].frame_bytes())
        .map_err(|_| BackendError::Storage)?;
    let prepared: RunFrame = serde_json::from_slice(prefix.frames()[1].frame_bytes())
        .map_err(|_| BackendError::Storage)?;
    let conclusion: RunFrame = serde_json::from_slice(prefix.frames()[2].frame_bytes())
        .map_err(|_| BackendError::Storage)?;
    first.validate().map_err(|_| BackendError::Storage)?;
    prepared.validate().map_err(|_| BackendError::Storage)?;
    conclusion.validate().map_err(|_| BackendError::Storage)?;
    let identity = backend.identity();
    crate::single_trust::QualifiedRun::qualify_prefix(
        identity.scope().clone(),
        identity.epoch(),
        identity.tenant().clone(),
        vec![first.clone(), prepared.clone(), conclusion.clone()],
    )
    .map_err(|_| BackendError::Storage)?;
    if first.expected_sequence() != 1
        || prepared.expected_sequence() != 2
        || !matches!(prepared.record(), RunRecord::StatePrepared(_))
        || conclusion.expected_sequence() != 3
        || !matches!(conclusion.record(), RunRecord::StateConcluded(_))
        || first.head_digest(None).map_err(|_| BackendError::Storage)?
            != *prefix.frames()[0].head_digest()
        || prepared
            .head_digest(Some(prefix.frames()[0].head_digest()))
            .map_err(|_| BackendError::Storage)?
            != *prefix.frames()[1].head_digest()
        || conclusion
            .head_digest(Some(prefix.frames()[1].head_digest()))
            .map_err(|_| BackendError::Storage)?
            != *prefix.frames()[2].head_digest()
    {
        return Err(BackendError::Conflict);
    }
    Ok(())
}

/// Appends one bounded sole-genesis probe for process-level CAS tests.
pub async fn append_admission_probe(
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
    run_id: RunId,
    append_request_id: &str,
) -> Result<BackendAppendOutcome, BackendError> {
    let frame = process_race_admission_frame(&identity, run_id, append_request_id)?;
    append_process_race_frame(&backend, &identity, &frame, None).await
}

fn process_race_admission_frame(
    identity: &StructuredStoreIdentity,
    run_id: RunId,
    append_request_id: &str,
) -> Result<RunFrame, BackendError> {
    let (input, input_object) = restart_probe_value(1, "process-admission-input")?;
    let program_ref = restart_probe_content(2, "process-admission-program")?;
    let configuration_ref = restart_probe_content(3, "process-admission-configuration")?;
    let admission = RunAdmitted::new(
        identity.scope().clone(),
        identity.epoch(),
        run_id.clone(),
        identity.tenant().clone(),
        StableId::new("mfm.test.process-race@1").map_err(|_| BackendError::Storage)?,
        program_ref,
        input,
        ConfigurationHeadProjection::new(1, configuration_ref)
            .map_err(|_| BackendError::Storage)?,
        Vec::new(),
    )
    .map_err(|_| BackendError::Storage)?;
    let frame = RunFrame::new(
        run_id,
        identity.scope().clone(),
        identity.epoch(),
        1,
        AppendRequestId::new(append_request_id).map_err(|_| BackendError::Storage)?,
        RunRecord::RunAdmitted(admission),
        vec![input_object],
    )
    .map_err(|_| BackendError::Storage)?;
    Ok(frame)
}

fn process_race_run_id(stage: ProcessRaceStage) -> Result<RunId, BackendError> {
    RunId::parse(match stage {
        ProcessRaceStage::Admission => {
            "run:sha256-jcs-v1:c123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
        ProcessRaceStage::Preparation => {
            "run:sha256-jcs-v1:e123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
        ProcessRaceStage::Replacement => {
            "run:sha256-jcs-v1:f123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
        ProcessRaceStage::Conclusion => {
            "run:sha256-jcs-v1:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
    })
    .map_err(|_| BackendError::Storage)
}

async fn append_process_race_frame(
    backend: &Arc<dyn StructuredStoreBackend>,
    identity: &StructuredStoreIdentity,
    frame: &RunFrame,
    previous_head: Option<&ContentDigest>,
) -> Result<BackendAppendOutcome, BackendError> {
    let bytes = frame.canonical_bytes().map_err(|_| BackendError::Storage)?;
    let frame_digest = raw_content_digest(bytes.as_bytes());
    let head_digest = frame
        .head_digest(previous_head)
        .map_err(|_| BackendError::Storage)?;
    let command = BackendAppendCommand::new(
        identity,
        frame.run_id(),
        frame.expected_sequence(),
        frame.append_request_id(),
        bytes.as_bytes(),
        &frame_digest,
        &head_digest,
        previous_head,
        frame.record().is_admission(),
        None,
    );
    backend.compare_and_append(&command).await
}

fn process_race_prepared_frame(
    identity: &StructuredStoreIdentity,
    run_id: &RunId,
    sequence: u64,
    ordinal: u16,
    replaces: Option<PreparationRef>,
    append_request_id: &str,
) -> Result<RunFrame, BackendError> {
    let (input, _) = restart_probe_value(11, "process-race-input")?;
    let (intent, intent_object) = restart_probe_value(12, "process-race-intent")?;
    let physical_target_ref = restart_probe_content(13, "process-race-target")?;
    let state_ref = restart_probe_content(14, "process-race-state")?;
    let capability_ref = restart_probe_content(15, "process-race-capability")?;
    let adapter_ref = restart_probe_content(16, "process-race-adapter")?;
    let binding = BindingDescriptor::new(
        state_ref,
        Some(capability_ref),
        Some(adapter_ref),
        physical_target_ref,
        None,
        None,
    )
    .map_err(|_| BackendError::Storage)?;
    let binding_ref = binding.content_ref().map_err(|_| BackendError::Storage)?;
    let prepared = StatePrepared::new(
        SequentialControlAddress::new(0, Vec::new()).map_err(|_| BackendError::Storage)?,
        ordinal,
        input,
        intent,
        None,
        None,
        PreparationMode::Read {
            total_attempt_bound: 2,
        },
        binding,
        binding_ref,
        replaces,
        4096,
    )
    .map_err(|_| BackendError::Storage)?;
    RunFrame::new(
        run_id.clone(),
        identity.scope().clone(),
        identity.epoch(),
        sequence,
        AppendRequestId::new(append_request_id).map_err(|_| BackendError::Storage)?,
        RunRecord::StatePrepared(prepared),
        vec![intent_object],
    )
    .map_err(|_| BackendError::Storage)
}

fn process_race_conclusion_frame(
    identity: &StructuredStoreIdentity,
    run_id: &RunId,
    sequence: u64,
    append_request_id: &str,
) -> Result<RunFrame, BackendError> {
    let (input, input_object) = restart_probe_value(21, "process-race-input")?;
    let (intent, intent_object) = restart_probe_value(22, "process-race-intent")?;
    let conclusion = StateConcluded::Access {
        occurrence: SequentialControlAddress::new(0, Vec::new())
            .map_err(|_| BackendError::Storage)?,
        preparation: PreparationRef::new(run_id.clone(), 2, 1),
        evidence: intent,
        outcome: StateOutcome::Success(input),
        fact_proposals: None,
        fact_selection: None,
        fact_publication: None,
    };
    RunFrame::new(
        run_id.clone(),
        identity.scope().clone(),
        identity.epoch(),
        sequence,
        AppendRequestId::new(append_request_id).map_err(|_| BackendError::Storage)?,
        RunRecord::StateConcluded(conclusion),
        vec![input_object, intent_object],
    )
    .map_err(|_| BackendError::Storage)
}

/// Seeds the prefix for one independent same-epoch process race.
pub async fn seed_process_race(
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
    stage: ProcessRaceStage,
) -> Result<(), BackendError> {
    let run_id = process_race_run_id(stage)?;
    if stage == ProcessRaceStage::Admission {
        return Ok(());
    }
    if !matches!(
        append_admission_probe(
            Arc::clone(&backend),
            identity.clone(),
            run_id.clone(),
            "process-race-seed-admission-0123456789",
        )
        .await?,
        BackendAppendOutcome::NewlyCommitted
    ) {
        return Err(BackendError::Conflict);
    }
    if stage == ProcessRaceStage::Preparation {
        return Ok(());
    }
    let frame = process_race_prepared_frame(
        &identity,
        &run_id,
        2,
        0,
        None,
        "process-race-seed-preparation-0123456789",
    )?;
    let prefix = backend
        .load_complete_prefix(&run_id, RawHistoryLoadLimit::new(4, 16 * 1024))
        .await?
        .ok_or(BackendError::Storage)?;
    let previous_head = prefix
        .frames()
        .last()
        .map(RawFrameBytes::head_digest)
        .ok_or(BackendError::Storage)?;
    if !matches!(
        append_process_race_frame(&backend, &identity, &frame, Some(previous_head)).await?,
        BackendAppendOutcome::NewlyCommitted
    ) {
        return Err(BackendError::Conflict);
    }
    Ok(())
}

/// Appends one competing candidate for an independent same-epoch process race.
pub async fn process_race_candidate_head(
    backend: Arc<dyn StructuredStoreBackend>,
    stage: ProcessRaceStage,
) -> Result<(u64, Option<ContentDigest>), BackendError> {
    let run_id = process_race_run_id(stage)?;
    let prefix = backend
        .load_complete_prefix(&run_id, RawHistoryLoadLimit::new(4, 16 * 1024))
        .await?;
    Ok(match prefix.as_ref() {
        Some(prefix) => (
            u64::try_from(prefix.frames().len() + 1).map_err(|_| BackendError::Capacity)?,
            prefix
                .frames()
                .last()
                .map(|frame| frame.head_digest().clone()),
        ),
        None => (1, None),
    })
}

/// Appends one process-race candidate using a previously observed exact head.
pub async fn append_process_race_candidate_at_head(
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
    stage: ProcessRaceStage,
    candidate: &str,
    sequence: u64,
    previous_head: Option<ContentDigest>,
) -> Result<BackendAppendOutcome, BackendError> {
    let run_id = process_race_run_id(stage)?;
    let append_request_id = format!("process-race-{}-{}-0123456789", stage.as_str(), candidate);
    if stage == ProcessRaceStage::Admission {
        let frame = process_race_admission_frame(&identity, run_id, &append_request_id)?;
        return append_process_race_frame(&backend, &identity, &frame, previous_head.as_ref())
            .await;
    }
    let frame = match stage {
        ProcessRaceStage::Preparation => {
            process_race_prepared_frame(&identity, &run_id, sequence, 0, None, &append_request_id)?
        }
        ProcessRaceStage::Replacement => process_race_prepared_frame(
            &identity,
            &run_id,
            sequence,
            1,
            Some(PreparationRef::new(run_id.clone(), 2, 1)),
            &append_request_id,
        )?,
        ProcessRaceStage::Conclusion => {
            process_race_conclusion_frame(&identity, &run_id, sequence, &append_request_id)?
        }
        ProcessRaceStage::Admission => unreachable!("admission handled above"),
    };
    append_process_race_frame(&backend, &identity, &frame, previous_head.as_ref()).await
}

/// Loads the current head and appends one competing candidate in one convenience call.
pub async fn append_process_race_candidate(
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
    stage: ProcessRaceStage,
    candidate: &str,
) -> Result<BackendAppendOutcome, BackendError> {
    let (sequence, previous_head) =
        process_race_candidate_head(Arc::clone(&backend), stage).await?;
    append_process_race_candidate_at_head(
        backend,
        identity,
        stage,
        candidate,
        sequence,
        previous_head,
    )
    .await
}

/// Verifies that exactly one candidate crossed the process-level CAS point.
pub async fn verify_process_race(
    backend: Arc<dyn StructuredStoreBackend>,
    stage: ProcessRaceStage,
) -> Result<(), BackendError> {
    let run_id = process_race_run_id(stage)?;
    let prefix = backend
        .load_complete_prefix(&run_id, RawHistoryLoadLimit::new(4, 16 * 1024))
        .await?
        .ok_or(BackendError::Storage)?;
    if prefix.frames().len() != stage.prefix_len() + 1 {
        return Err(BackendError::Conflict);
    }
    let last = prefix.frames().last().ok_or(BackendError::Storage)?;
    let frame: RunFrame =
        serde_json::from_slice(last.frame_bytes()).map_err(|_| BackendError::Storage)?;
    frame.validate().map_err(|_| BackendError::Storage)?;
    let valid_record = match stage {
        ProcessRaceStage::Admission => matches!(frame.record(), RunRecord::RunAdmitted(_)),
        ProcessRaceStage::Preparation => matches!(
            frame.record(),
            RunRecord::StatePrepared(prepared) if prepared.preparation_ordinal() == 0
        ),
        ProcessRaceStage::Replacement => matches!(
            frame.record(),
            RunRecord::StatePrepared(prepared)
                if prepared.preparation_ordinal() == 1
                    && prepared.replaces().is_some_and(|replacement| {
                        replacement.run_sequence() == 2 && replacement.record_ordinal() == 1
                    })
        ),
        ProcessRaceStage::Conclusion => matches!(
            frame.record(),
            RunRecord::StateConcluded(StateConcluded::Access { preparation, .. })
                if preparation.run_sequence() == 2 && preparation.record_ordinal() == 1
        ),
    };
    if !valid_record
        || !last
            .append_request_id()
            .as_str()
            .starts_with("process-race-")
    {
        return Err(BackendError::Conflict);
    }
    Ok(())
}

/// Exercises the complete mechanical contract without decoding or reducing any frame.
pub async fn exercise(
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
) -> Result<(), BackendError> {
    if backend.identity() != identity {
        return Err(BackendError::Identity);
    }

    let run_id = RunId::parse(
        "run:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .map_err(|_| BackendError::Storage)?;
    let first_bytes = br#"{"kind":"conformance","sequence":1}"#;
    let first_digest = raw_content_digest(first_bytes);
    let first_head = ContentDigest::parse(
        "content:sha256-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .map_err(|_| BackendError::Storage)?;
    let first_request = AppendRequestId::new("backend-conformance-first-0123456789ab")
        .map_err(|_| BackendError::Storage)?;
    let first = BackendAppendCommand::new(
        &identity,
        &run_id,
        1,
        &first_request,
        first_bytes,
        &first_digest,
        &first_head,
        None,
        true,
        None,
    );
    assert_eq!(
        backend.compare_and_append(&first).await?,
        BackendAppendOutcome::NewlyCommitted
    );

    let retry = BackendAppendCommand::new(
        &identity,
        &run_id,
        1,
        &first_request,
        first_bytes,
        &first_digest,
        &first_head,
        None,
        true,
        None,
    );
    match backend.compare_and_append(&retry).await? {
        BackendAppendOutcome::Found(found) => {
            assert_eq!(found.sequence(), 1);
            assert_eq!(found.frame_bytes(), first_bytes);
        }
        other => panic!("unexpected idempotent backend outcome: {other:?}"),
    }

    let stale_request = AppendRequestId::new("backend-conformance-stale-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let stale = BackendAppendCommand::new(
        &identity,
        &run_id,
        1,
        &stale_request,
        first_bytes,
        &first_digest,
        &first_head,
        None,
        true,
        None,
    );
    assert!(matches!(
        backend.compare_and_append(&stale).await?,
        BackendAppendOutcome::StaleHead { actual_sequence: 1 }
    ));

    let second_bytes = br#"{"kind":"conformance","sequence":2}"#;
    let second_digest = raw_content_digest(second_bytes);
    let second_head = ContentDigest::parse(
        "content:sha256-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .map_err(|_| BackendError::Storage)?;
    let second_request = AppendRequestId::new("backend-conformance-race-a-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let second = BackendAppendCommand::new(
        &identity,
        &run_id,
        2,
        &second_request,
        second_bytes,
        &second_digest,
        &second_head,
        Some(&first_head),
        false,
        None,
    );
    let third_bytes = br#"{"kind":"conformance","sequence":3}"#;
    let third_digest = raw_content_digest(third_bytes);
    let third_head = ContentDigest::parse(
        "content:sha256-v1:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    )
    .map_err(|_| BackendError::Storage)?;
    let third_request = AppendRequestId::new("backend-conformance-race-b-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let third = BackendAppendCommand::new(
        &identity,
        &run_id,
        2,
        &third_request,
        third_bytes,
        &third_digest,
        &third_head,
        Some(&first_head),
        false,
        None,
    );
    let (left, right) = tokio::join!(
        backend.compare_and_append(&second),
        backend.compare_and_append(&third)
    );
    let left = left?;
    let right = right?;
    assert!(matches!(
        (&left, &right),
        (
            BackendAppendOutcome::NewlyCommitted,
            BackendAppendOutcome::StaleHead { actual_sequence: 2 }
        ) | (
            BackendAppendOutcome::StaleHead { actual_sequence: 2 },
            BackendAppendOutcome::NewlyCommitted
        )
    ));

    let prefix = backend
        .load_complete_prefix(&run_id, RawHistoryLoadLimit::new(4, 1024))
        .await?
        .ok_or(BackendError::Storage)?;
    assert_eq!(prefix.frames().len(), 2);
    assert_eq!(prefix.frames()[0].frame_digest(), &first_digest);

    let schema = SchemaId::new(
        "mfm.backend-conformance",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_hex("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .map_err(|_| BackendError::Storage)?,
    )
    .map_err(|_| BackendError::Storage)?;
    let configuration_bytes = br#"{"mode":"conformance"}"#;
    let configuration_ref = ContentRef::new(schema, raw_content_digest(configuration_bytes))
        .map_err(|_| BackendError::Storage)?;
    let configuration_request = AppendRequestId::new("backend-conformance-config-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let configuration = ConfigurationAppendCommand::new(
        &identity,
        0,
        &configuration_request,
        configuration_bytes,
        &configuration_ref,
    );
    assert_eq!(
        backend
            .compare_and_append_configuration(&configuration)
            .await?,
        BackendConfigurationOutcome::NewlyCommitted
    );
    assert!(matches!(
        backend
            .compare_and_append_configuration(&configuration)
            .await?,
        BackendConfigurationOutcome::Found(revision) if revision.sequence() == 1
    ));

    let configuration_schema = SchemaId::new(
        "mfm.backend-conformance.configuration-race",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| BackendError::Storage)?;
    let left_configuration_bytes = br#"{"mode":"configuration-race-left"}"#;
    let left_configuration_ref = ContentRef::new(
        configuration_schema.clone(),
        raw_content_digest(left_configuration_bytes),
    )
    .map_err(|_| BackendError::Storage)?;
    let right_configuration_bytes = br#"{"mode":"configuration-race-right"}"#;
    let right_configuration_ref = ContentRef::new(
        configuration_schema,
        raw_content_digest(right_configuration_bytes),
    )
    .map_err(|_| BackendError::Storage)?;
    let left_configuration_request =
        AppendRequestId::new("backend-conformance-config-race-left-012345")
            .map_err(|_| BackendError::Storage)?;
    let right_configuration_request =
        AppendRequestId::new("backend-conformance-config-race-right-012345")
            .map_err(|_| BackendError::Storage)?;
    let left_configuration = ConfigurationAppendCommand::new(
        &identity,
        1,
        &left_configuration_request,
        left_configuration_bytes,
        &left_configuration_ref,
    );
    let right_configuration = ConfigurationAppendCommand::new(
        &identity,
        1,
        &right_configuration_request,
        right_configuration_bytes,
        &right_configuration_ref,
    );
    let (left, right) = tokio::join!(
        backend.compare_and_append_configuration(&left_configuration),
        backend.compare_and_append_configuration(&right_configuration)
    );
    let left = left?;
    let right = right?;
    assert!(matches!(
        (&left, &right),
        (
            BackendConfigurationOutcome::NewlyCommitted,
            BackendConfigurationOutcome::StaleHead { actual_sequence: 2 }
        ) | (
            BackendConfigurationOutcome::StaleHead { actual_sequence: 2 },
            BackendConfigurationOutcome::NewlyCommitted
        )
    ));
    let configuration_rows = backend.load_configuration().await?;
    assert_eq!(configuration_rows.len(), 2);
    assert_eq!(configuration_rows[0].canonical_bytes(), configuration_bytes);
    assert_eq!(
        configuration_rows[0].total_bytes(),
        configuration_bytes.len()
    );
    assert_eq!(
        configuration_rows[1].total_bytes(),
        configuration_bytes.len()
            + if matches!(left, BackendConfigurationOutcome::NewlyCommitted) {
                left_configuration_bytes.len()
            } else {
                right_configuration_bytes.len()
            }
    );

    let facts = backend.load_facts().await?;
    assert_eq!(facts, RawFactSnapshot::new(0, Vec::new())?);
    let run_ids = backend.audit_run_ids().await?;
    assert_eq!(run_ids, vec![run_id]);

    assert!(RawFrameBytes::new(
        0,
        first_request,
        first_bytes.to_vec(),
        first_digest,
        first_head,
    )
    .is_err());
    assert!(RawRunPrefix::new(Vec::new()).is_err());
    assert!(RawConfigurationRevision::new(
        0,
        configuration_request,
        configuration_bytes.to_vec(),
        configuration_ref,
        configuration_bytes.len(),
    )
    .is_err());

    Ok(())
}

/// Races the first fact publication for two independent runs sharing one empty fact head.
pub async fn exercise_first_fact_publication_race(
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
) -> Result<(), BackendError> {
    let left_run = RunId::parse(
        "run:sha256-jcs-v1:b123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .map_err(|_| BackendError::Storage)?;
    let right_run = RunId::parse(
        "run:sha256-jcs-v1:d123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .map_err(|_| BackendError::Storage)?;
    let left_first_bytes = br#"{"kind":"fact-race-left-admission"}"#;
    let right_first_bytes = br#"{"kind":"fact-race-right-admission"}"#;
    let left_first_digest = raw_content_digest(left_first_bytes);
    let right_first_digest = raw_content_digest(right_first_bytes);
    let left_first_head = ContentDigest::parse(
        "content:sha256-v1:1212121212121212121212121212121212121212121212121212121212121212",
    )
    .map_err(|_| BackendError::Storage)?;
    let right_first_head = ContentDigest::parse(
        "content:sha256-v1:1313131313131313131313131313131313131313131313131313131313131313",
    )
    .map_err(|_| BackendError::Storage)?;
    let left_first_request = AppendRequestId::new("fact-race-left-admission-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let right_first_request = AppendRequestId::new("fact-race-right-admission-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let left_first = BackendAppendCommand::new(
        &identity,
        &left_run,
        1,
        &left_first_request,
        left_first_bytes,
        &left_first_digest,
        &left_first_head,
        None,
        true,
        None,
    );
    let right_first = BackendAppendCommand::new(
        &identity,
        &right_run,
        1,
        &right_first_request,
        right_first_bytes,
        &right_first_digest,
        &right_first_head,
        None,
        true,
        None,
    );
    if !matches!(
        backend.compare_and_append(&left_first).await?,
        BackendAppendOutcome::NewlyCommitted
    ) || !matches!(
        backend.compare_and_append(&right_first).await?,
        BackendAppendOutcome::NewlyCommitted
    ) {
        return Err(BackendError::Conflict);
    }

    let proposal_schema = SchemaId::new(
        "mfm.test.fact-race-proposals",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([4; 32]),
    )
    .map_err(|_| BackendError::Storage)?;
    let proposal_ref = ContentRef::new(proposal_schema, raw_content_digest(b"fact-race-proposals"))
        .map_err(|_| BackendError::Storage)?;
    let left_publication = RawFactPublication::new(1, left_run.clone(), 2, proposal_ref.clone())?;
    let right_publication = RawFactPublication::new(1, right_run.clone(), 2, proposal_ref)?;
    let left_second_bytes = br#"{"kind":"fact-race-left-publication"}"#;
    let right_second_bytes = br#"{"kind":"fact-race-right-publication"}"#;
    let left_second_digest = raw_content_digest(left_second_bytes);
    let right_second_digest = raw_content_digest(right_second_bytes);
    let left_second_head = ContentDigest::parse(
        "content:sha256-v1:1414141414141414141414141414141414141414141414141414141414141414",
    )
    .map_err(|_| BackendError::Storage)?;
    let right_second_head = ContentDigest::parse(
        "content:sha256-v1:1515151515151515151515151515151515151515151515151515151515151515",
    )
    .map_err(|_| BackendError::Storage)?;
    let left_second_request = AppendRequestId::new("fact-race-left-publication-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let right_second_request = AppendRequestId::new("fact-race-right-publication-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let left_second = BackendAppendCommand::new(
        &identity,
        &left_run,
        2,
        &left_second_request,
        left_second_bytes,
        &left_second_digest,
        &left_second_head,
        Some(&left_first_head),
        false,
        Some(&left_publication),
    );
    let right_second = BackendAppendCommand::new(
        &identity,
        &right_run,
        2,
        &right_second_request,
        right_second_bytes,
        &right_second_digest,
        &right_second_head,
        Some(&right_first_head),
        false,
        Some(&right_publication),
    );
    let (left, right) = tokio::join!(
        backend.compare_and_append(&left_second),
        backend.compare_and_append(&right_second)
    );
    let committed = usize::from(matches!(&left, Ok(BackendAppendOutcome::NewlyCommitted)))
        + usize::from(matches!(&right, Ok(BackendAppendOutcome::NewlyCommitted)));
    let frontier_losers = usize::from(matches!(&left, Err(BackendError::FactFrontierChanged)))
        + usize::from(matches!(&right, Err(BackendError::FactFrontierChanged)));
    if committed != 1 || frontier_losers != 1 {
        return Err(BackendError::Conflict);
    }
    if backend.load_facts().await?.head_sequence() != 1 {
        return Err(BackendError::Conflict);
    }
    let left_prefix = backend
        .load_complete_prefix(&left_run, RawHistoryLoadLimit::new(4, 4096))
        .await?
        .ok_or(BackendError::Storage)?;
    let right_prefix = backend
        .load_complete_prefix(&right_run, RawHistoryLoadLimit::new(4, 4096))
        .await?
        .ok_or(BackendError::Storage)?;
    if left_prefix.frames().len() + right_prefix.frames().len() != 3 {
        return Err(BackendError::Conflict);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemoryStructuredBackend;
    use mfm_ids::{StoreEpoch, StoreScopeId, TenantScopeId};

    #[tokio::test]
    async fn memory_backend_satisfies_shared_contract() {
        let identity = StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            StoreEpoch::new(1),
            TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        );
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        exercise(backend, identity).await.expect("backend contract");
    }

    #[tokio::test]
    async fn memory_backend_serializes_first_fact_publication() {
        let identity = StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            StoreEpoch::new(1),
            TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        );
        let backend: Arc<dyn StructuredStoreBackend> =
            Arc::new(MemoryStructuredBackend::new(identity.clone()));
        exercise_first_fact_publication_race(backend, identity)
            .await
            .expect("first fact publication race");
    }
}
