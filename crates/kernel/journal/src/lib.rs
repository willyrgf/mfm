#![warn(missing_docs)]
//! Exact canonical run frames and qualified append-only history.
//!
//! Journal owns the one RunFrameV2 encoder and qualifier. Store moves opaque
//! bytes; Runtime receives only borrowed qualified record and object views.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mfm_canonical::{raw_content_digest, sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, EffectId, RunId};
use mfm_values::MAX_RUN_OBJECT_CANONICAL_BYTES;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

/// Maximum canonical bytes in one frame.
pub const MAX_FRAME_BYTES: usize = 25_231_360;
/// Maximum measured frame bytes outside embedded object values.
pub const MAX_FRAME_NON_PAYLOAD_ENVELOPE: usize = 65_536;
/// Maximum frames in one run.
pub const MAX_RUN_FRAMES: u64 = 65_536;
/// Maximum canonical frame bytes in one run.
pub const MAX_RUN_BYTES: u64 = 536_870_912;

/// Result type for Journal operations.
type Result<T> = std::result::Result<T, JournalError>;

/// Redaction-safe Journal failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    /// A fixed frame, object, count, or run-byte ceiling was exceeded.
    #[error("journal capacity exceeded")]
    Capacity,
    /// A locally constructed frame violated the sealed frame contract.
    #[error("journal frame is invalid")]
    InvalidFrame,
    /// Retained bytes did not form one exact qualified history.
    #[error("journal history is invalid")]
    InvalidHistory,
}

/// Outcome branch recorded by a State conclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    /// Successful State outcome.
    Success,
    /// Failed State outcome.
    Failure,
}

/// Derives the exact-byte recursive frame head without qualifying the input.
pub fn frame_head_digest(frame_bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(frame_bytes))
}

/// Borrowed view of one frame-local canonical object.
pub struct JournalObject<'a> {
    content_ref: &'a ContentRef,
    canonical: &'a [u8],
}

impl<'a> JournalObject<'a> {
    /// Returns the complete object content reference.
    pub const fn content_ref(&self) -> &ContentRef {
        self.content_ref
    }

    /// Returns exact qualified canonical object bytes.
    pub const fn canonical_bytes(&self) -> &[u8] {
        self.canonical
    }
}

impl fmt::Debug for JournalObject<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JournalObject")
            .field("content_ref", self.content_ref)
            .field("canonical", &"<redacted>")
            .finish()
    }
}

/// Borrowed qualified semantic record.
#[derive(Debug)]
pub enum JournalRecord<'a> {
    /// Genesis admission with the exact Program and C0.
    RunAdmitted {
        /// Retained Program object.
        program: JournalObject<'a>,
        /// Retained admitted-context object.
        admitted_context: JournalObject<'a>,
    },
    /// One fused Pure conclusion.
    StateConcludedPure {
        /// Success or failure branch.
        kind: OutcomeKind,
        /// Complete typed outcome.
        outcome: JournalObject<'a>,
    },
    /// One fused Read observation and conclusion.
    StateConcludedRead {
        /// Exact prepared intent.
        intent: JournalObject<'a>,
        /// Exact accepted evidence.
        evidence: JournalObject<'a>,
        /// Success or failure branch.
        kind: OutcomeKind,
        /// Complete typed outcome.
        outcome: JournalObject<'a>,
    },
    /// One durable Effect command prepared before adapter entry.
    StateEffectPrepared {
        /// Durable identity derived from the exact occurrence and command.
        effect_id: &'a EffectId,
        /// Exact prepared command.
        command: JournalObject<'a>,
    },
    /// One Effect conclusion adjacent to its prepare.
    StateEffectConcluded {
        /// Exact accepted evidence.
        evidence: JournalObject<'a>,
        /// Success or failure branch.
        kind: OutcomeKind,
        /// Complete typed outcome.
        outcome: JournalObject<'a>,
    },
}

/// One sealed valid canonical frame.
pub struct EncodedRunFrame {
    frame: QualifiedFrame,
}

impl fmt::Debug for EncodedRunFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodedRunFrame")
            .field("run_id", &self.frame.run_id)
            .field("run_sequence", &self.frame.run_sequence)
            .field("head_digest", &self.frame.head_digest)
            .field("canonical_bytes", &"<redacted>")
            .finish()
    }
}

impl EncodedRunFrame {
    /// Constructs the genesis admission frame.
    pub fn admission(
        run_id: &RunId,
        program_ref: &ContentRef,
        program: &[u8],
        context_ref: &ContentRef,
        context: &[u8],
    ) -> std::result::Result<Self, JournalError> {
        construct_frame(
            run_id.clone(),
            1,
            None,
            Record::RunAdmitted {
                program_ref: program_ref.clone(),
                admitted_context: context_ref.clone(),
            },
            vec![
                (program_ref.clone(), program),
                (context_ref.clone(), context),
            ],
        )
    }

    /// Returns the run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.frame.run_id
    }

    /// Returns the one-based frame sequence.
    pub const fn run_sequence(&self) -> u64 {
        self.frame.run_sequence
    }

    /// Returns the predecessor head, absent only at genesis.
    pub const fn previous_head_digest(&self) -> Option<&ContentDigest> {
        self.frame.previous_head_digest.as_ref()
    }

    /// Returns the exact-byte digest of this frame.
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.frame.head_digest
    }

    /// Returns exact canonical RunFrameV2 bytes.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.frame.canonical.as_bytes()
    }
}

/// Opaque unqualified Store-to-Journal transfer.
pub struct StoredRunBytes {
    ordered_frames: Vec<Vec<u8>>,
}

impl fmt::Debug for StoredRunBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredRunBytes")
            .field("frame_count", &self.ordered_frames.len())
            .field("bytes", &"<redacted>")
            .finish()
    }
}

impl StoredRunBytes {
    /// Constructs one bounded, nonempty, still-unqualified ordered transfer.
    pub fn new(ordered_frames: Vec<Vec<u8>>) -> std::result::Result<Self, JournalError> {
        validate_transfer_lengths(ordered_frames.len(), ordered_frames.iter().map(Vec::len))?;
        Ok(Self { ordered_frames })
    }
}

fn validate_transfer_lengths(
    frame_count: usize,
    lengths: impl IntoIterator<Item = usize>,
) -> std::result::Result<(), JournalError> {
    if frame_count == 0 {
        return Err(JournalError::InvalidHistory);
    }
    if u64::try_from(frame_count).map_err(|_| JournalError::Capacity)? > MAX_RUN_FRAMES {
        return Err(JournalError::Capacity);
    }
    let mut total = 0_u64;
    for frame_len in lengths {
        if frame_len > MAX_FRAME_BYTES {
            return Err(JournalError::Capacity);
        }
        total = total
            .checked_add(u64::try_from(frame_len).map_err(|_| JournalError::Capacity)?)
            .ok_or(JournalError::Capacity)?;
        if total > MAX_RUN_BYTES {
            return Err(JournalError::Capacity);
        }
    }
    Ok(())
}

/// One qualified complete nonempty run prefix.
pub struct JournalHistory {
    frames: Vec<QualifiedFrame>,
    total_bytes: u64,
}

impl fmt::Debug for JournalHistory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JournalHistory")
            .field("run_id", &self.run_id())
            .field("head_sequence", &self.head_sequence())
            .field("head_digest", &self.head_digest())
            .field("frame_count", &self.frames.len())
            .field("canonical_bytes", &"<redacted>")
            .finish()
    }
}

impl JournalHistory {
    /// Qualifies an opaque complete Store prefix for the requested RunId.
    pub fn qualify(
        expected_run_id: &RunId,
        stored: StoredRunBytes,
    ) -> std::result::Result<Self, JournalError> {
        let mut frames: Vec<QualifiedFrame> = Vec::with_capacity(stored.ordered_frames.len());
        let mut total = 0_u64;
        for (offset, bytes) in stored.ordered_frames.into_iter().enumerate() {
            total = total
                .checked_add(u64::try_from(bytes.len()).map_err(|_| JournalError::InvalidHistory)?)
                .ok_or(JournalError::InvalidHistory)?;
            if total > MAX_RUN_BYTES {
                return Err(JournalError::InvalidHistory);
            }
            let frame = qualify_frame(&bytes)?;
            let expected_sequence =
                u64::try_from(offset + 1).map_err(|_| JournalError::InvalidHistory)?;
            if frame.run_id != *expected_run_id || frame.run_sequence != expected_sequence {
                return Err(JournalError::InvalidHistory);
            }
            if offset == 0 {
                if frame.previous_head_digest.is_some()
                    || !matches!(frame.record, Record::RunAdmitted { .. })
                {
                    return Err(JournalError::InvalidHistory);
                }
            } else {
                let previous = frames.last().ok_or(JournalError::InvalidHistory)?;
                if frame.previous_head_digest.as_ref() != Some(&previous.head_digest)
                    || matches!(frame.record, Record::RunAdmitted { .. })
                    || !records_are_adjacent(&previous.record, &frame.record)
                {
                    return Err(JournalError::InvalidHistory);
                }
            }
            frames.push(frame);
        }
        Ok(Self {
            frames,
            total_bytes: total,
        })
    }

    /// Starts a qualified history from one known genesis.
    pub fn from_genesis(frame: EncodedRunFrame) -> std::result::Result<Self, JournalError> {
        if frame.run_sequence() != 1
            || frame.previous_head_digest().is_some()
            || !matches!(frame.frame.record, Record::RunAdmitted { .. })
        {
            return Err(JournalError::InvalidFrame);
        }
        let total_bytes =
            u64::try_from(frame.canonical_bytes().len()).map_err(|_| JournalError::Capacity)?;
        Ok(Self {
            frames: vec![frame.frame],
            total_bytes,
        })
    }

    /// Encodes a fused Pure conclusion against this exact head.
    pub fn encode_pure_conclusion(
        &self,
        kind: OutcomeKind,
        value_ref: &ContentRef,
        value: &[u8],
    ) -> std::result::Result<EncodedRunFrame, JournalError> {
        self.construct_successor(
            Record::StateConcludedPure {
                outcome: Outcome::new(kind, value_ref.clone()),
            },
            vec![(value_ref.clone(), value)],
        )
    }

    /// Encodes a fused Read conclusion against this exact head.
    #[allow(clippy::too_many_arguments)]
    pub fn encode_read_conclusion(
        &self,
        intent_ref: &ContentRef,
        intent: &[u8],
        evidence_ref: &ContentRef,
        evidence: &[u8],
        kind: OutcomeKind,
        value_ref: &ContentRef,
        value: &[u8],
    ) -> std::result::Result<EncodedRunFrame, JournalError> {
        self.construct_successor(
            Record::StateConcludedRead {
                intent: intent_ref.clone(),
                evidence: evidence_ref.clone(),
                outcome: Outcome::new(kind, value_ref.clone()),
            },
            vec![
                (intent_ref.clone(), intent),
                (evidence_ref.clone(), evidence),
                (value_ref.clone(), value),
            ],
        )
    }

    /// Encodes one Effect prepare against this exact head.
    pub fn encode_effect_prepare(
        &self,
        effect_id: &EffectId,
        command_ref: &ContentRef,
        command: &[u8],
    ) -> std::result::Result<EncodedRunFrame, JournalError> {
        self.construct_successor(
            Record::StateEffectPrepared {
                effect_id: effect_id.clone(),
                command: command_ref.clone(),
            },
            vec![(command_ref.clone(), command)],
        )
    }

    /// Encodes one Effect conclusion adjacent to this history's pending prepare.
    pub fn encode_effect_conclusion(
        &self,
        evidence_ref: &ContentRef,
        evidence: &[u8],
        kind: OutcomeKind,
        value_ref: &ContentRef,
        value: &[u8],
    ) -> std::result::Result<EncodedRunFrame, JournalError> {
        self.construct_successor(
            Record::StateEffectConcluded {
                evidence: evidence_ref.clone(),
                outcome: Outcome::new(kind, value_ref.clone()),
            },
            vec![(evidence_ref.clone(), evidence), (value_ref.clone(), value)],
        )
    }

    fn construct_successor(
        &self,
        record: Record,
        objects: Vec<(ContentRef, &[u8])>,
    ) -> std::result::Result<EncodedRunFrame, JournalError> {
        let previous = &self.frames[self.frames.len() - 1].record;
        if !records_are_adjacent(previous, &record) {
            return Err(JournalError::InvalidFrame);
        }
        let sequence = self
            .head_sequence()
            .checked_add(1)
            .ok_or(JournalError::Capacity)?;
        if sequence > MAX_RUN_FRAMES {
            return Err(JournalError::Capacity);
        }
        let frame = construct_frame(
            self.run_id().clone(),
            sequence,
            Some(self.head_digest().clone()),
            record,
            objects,
        )?;
        let candidate =
            u64::try_from(frame.canonical_bytes().len()).map_err(|_| JournalError::Capacity)?;
        let remaining = MAX_RUN_BYTES
            .checked_sub(self.total_bytes)
            .ok_or(JournalError::Capacity)?;
        if candidate > remaining {
            return Err(JournalError::Capacity);
        }
        Ok(frame)
    }

    /// Extends this accumulator with one known-inserted exact successor.
    pub fn extend_inserted(
        &mut self,
        inserted: EncodedRunFrame,
    ) -> std::result::Result<JournalRecord<'_>, JournalError> {
        let expected_sequence = self
            .head_sequence()
            .checked_add(1)
            .ok_or(JournalError::InvalidFrame)?;
        if inserted.run_id() != self.run_id()
            || inserted.run_sequence() != expected_sequence
            || inserted.previous_head_digest() != Some(self.head_digest())
            || !records_are_adjacent(
                &self.frames[self.frames.len() - 1].record,
                &inserted.frame.record,
            )
        {
            return Err(JournalError::InvalidFrame);
        }
        let candidate =
            u64::try_from(inserted.canonical_bytes().len()).map_err(|_| JournalError::Capacity)?;
        let remaining = MAX_RUN_BYTES
            .checked_sub(self.total_bytes)
            .ok_or(JournalError::Capacity)?;
        if candidate > remaining {
            return Err(JournalError::Capacity);
        }
        self.total_bytes += candidate;
        self.frames.push(inserted.frame);
        self.frames
            .last()
            .map(QualifiedFrame::record_view)
            .ok_or(JournalError::InvalidFrame)
    }

    /// Iterates over borrowed qualified records.
    pub fn records(&self) -> impl ExactSizeIterator<Item = JournalRecord<'_>> + '_ {
        self.frames.iter().map(QualifiedFrame::record_view)
    }

    /// Returns the run identity.
    pub fn run_id(&self) -> &RunId {
        &self.frames[0].run_id
    }

    /// Returns the current frame sequence.
    pub fn head_sequence(&self) -> u64 {
        self.frames
            .last()
            .map(|frame| frame.run_sequence)
            .unwrap_or(0)
    }

    /// Returns the current exact-byte frame head.
    pub fn head_digest(&self) -> &ContentDigest {
        &self.frames[self.frames.len() - 1].head_digest
    }
}

struct QualifiedFrame {
    run_id: RunId,
    run_sequence: u64,
    previous_head_digest: Option<ContentDigest>,
    record: Record,
    objects: Vec<ObjectOwned>,
    canonical: PlainCanonicalJsonBytes,
    head_digest: ContentDigest,
}

impl QualifiedFrame {
    fn record_view(&self) -> JournalRecord<'_> {
        match &self.record {
            Record::RunAdmitted {
                program_ref,
                admitted_context,
            } => JournalRecord::RunAdmitted {
                program: self.object(program_ref),
                admitted_context: self.object(admitted_context),
            },
            Record::StateConcludedPure { outcome } => JournalRecord::StateConcludedPure {
                kind: outcome.kind(),
                outcome: self.object(outcome.value()),
            },
            Record::StateConcludedRead {
                intent,
                evidence,
                outcome,
            } => JournalRecord::StateConcludedRead {
                intent: self.object(intent),
                evidence: self.object(evidence),
                kind: outcome.kind(),
                outcome: self.object(outcome.value()),
            },
            Record::StateEffectPrepared { effect_id, command } => {
                JournalRecord::StateEffectPrepared {
                    effect_id,
                    command: self.object(command),
                }
            }
            Record::StateEffectConcluded { evidence, outcome } => {
                JournalRecord::StateEffectConcluded {
                    evidence: self.object(evidence),
                    kind: outcome.kind(),
                    outcome: self.object(outcome.value()),
                }
            }
        }
    }

    fn object(&self, content_ref: &ContentRef) -> JournalObject<'_> {
        let object = self
            .objects
            .iter()
            .find(|object| &object.content_ref == content_ref)
            .expect("qualified frame closure contains every record reference");
        JournalObject {
            content_ref: &object.content_ref,
            canonical: object.canonical.as_bytes(),
        }
    }
}

struct ObjectOwned {
    content_ref: ContentRef,
    canonical: PlainCanonicalJsonBytes,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Record {
    RunAdmitted {
        program_ref: ContentRef,
        admitted_context: ContentRef,
    },
    StateConcludedPure {
        outcome: Outcome,
    },
    StateConcludedRead {
        intent: ContentRef,
        evidence: ContentRef,
        outcome: Outcome,
    },
    StateEffectPrepared {
        effect_id: EffectId,
        command: ContentRef,
    },
    StateEffectConcluded {
        evidence: ContentRef,
        outcome: Outcome,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Outcome {
    Success { value: ContentRef },
    Failure { value: ContentRef },
}

impl Outcome {
    fn new(kind: OutcomeKind, value: ContentRef) -> Self {
        match kind {
            OutcomeKind::Success => Self::Success { value },
            OutcomeKind::Failure => Self::Failure { value },
        }
    }

    fn kind(&self) -> OutcomeKind {
        match self {
            Self::Success { .. } => OutcomeKind::Success,
            Self::Failure { .. } => OutcomeKind::Failure,
        }
    }

    fn value(&self) -> &ContentRef {
        match self {
            Self::Success { value } | Self::Failure { value } => value,
        }
    }
}

fn construct_frame(
    run_id: RunId,
    run_sequence: u64,
    previous_head_digest: Option<ContentDigest>,
    record: Record,
    raw_objects: Vec<(ContentRef, &[u8])>,
) -> std::result::Result<EncodedRunFrame, JournalError> {
    if run_sequence == 0
        || run_sequence > MAX_RUN_FRAMES
        || (run_sequence == 1) != previous_head_digest.is_none()
        || previous_head_digest
            .as_ref()
            .is_some_and(|digest| digest.algorithm() != DigestAlgorithm::Sha256V1)
    {
        return Err(JournalError::InvalidFrame);
    }
    let objects = qualify_local_objects(raw_objects)?;
    validate_closure(&record, &objects).map_err(|_| JournalError::InvalidFrame)?;
    let canonical = encode_frame(
        &run_id,
        run_sequence,
        previous_head_digest.as_ref(),
        &record,
        &objects,
    )
    .map_err(|error| match error {
        JournalError::Capacity => JournalError::Capacity,
        _ => JournalError::InvalidFrame,
    })?;
    let head_digest = frame_head_digest(canonical.as_bytes());
    Ok(EncodedRunFrame {
        frame: QualifiedFrame {
            run_id,
            run_sequence,
            previous_head_digest,
            record,
            objects,
            canonical,
            head_digest,
        },
    })
}

fn qualify_local_objects(
    raw_objects: Vec<(ContentRef, &[u8])>,
) -> std::result::Result<Vec<ObjectOwned>, JournalError> {
    let mut objects: BTreeMap<ContentRef, PlainCanonicalJsonBytes> = BTreeMap::new();
    for (content_ref, bytes) in raw_objects {
        if bytes.len() > MAX_RUN_OBJECT_CANONICAL_BYTES {
            return Err(JournalError::Capacity);
        }
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| JournalError::InvalidFrame)?;
        if content_ref.content_digest() != &raw_content_digest(canonical.as_bytes()) {
            return Err(JournalError::InvalidFrame);
        }
        if let Some(previous) = objects.get(&content_ref) {
            if previous.as_bytes() != canonical.as_bytes() {
                return Err(JournalError::InvalidFrame);
            }
        } else {
            objects.insert(content_ref, canonical);
        }
    }
    Ok(objects
        .into_iter()
        .map(|(content_ref, canonical)| ObjectOwned {
            content_ref,
            canonical,
        })
        .collect())
}

fn validate_closure(record: &Record, objects: &[ObjectOwned]) -> Result<()> {
    let required = record_refs(record);
    let actual = objects
        .iter()
        .map(|object| &object.content_ref)
        .collect::<BTreeSet<_>>();
    if required != actual {
        return Err(JournalError::InvalidFrame);
    }
    Ok(())
}

fn record_refs(record: &Record) -> BTreeSet<&ContentRef> {
    match record {
        Record::RunAdmitted {
            program_ref,
            admitted_context,
        } => BTreeSet::from([program_ref, admitted_context]),
        Record::StateConcludedPure { outcome } => BTreeSet::from([outcome.value()]),
        Record::StateConcludedRead {
            intent,
            evidence,
            outcome,
        } => BTreeSet::from([intent, evidence, outcome.value()]),
        Record::StateEffectPrepared { command, .. } => BTreeSet::from([command]),
        Record::StateEffectConcluded { evidence, outcome } => {
            BTreeSet::from([evidence, outcome.value()])
        }
    }
}

fn encode_frame(
    run_id: &RunId,
    run_sequence: u64,
    previous_head_digest: Option<&ContentDigest>,
    record: &Record,
    objects: &[ObjectOwned],
) -> Result<PlainCanonicalJsonBytes> {
    let wire_objects = objects
        .iter()
        .map(|object| {
            let text = std::str::from_utf8(object.canonical.as_bytes())
                .map_err(|_| JournalError::InvalidFrame)?;
            Ok(ObjectWire {
                content_ref: object.content_ref.clone(),
                canonical: RawValue::from_string(text.to_owned())
                    .map_err(|_| JournalError::InvalidFrame)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let wire = FrameWire {
        domain: "mfm.run.frame.v2".to_owned(),
        run_id: run_id.clone(),
        run_sequence,
        previous_head_digest: previous_head_digest.cloned(),
        record: record.clone(),
        objects: wire_objects,
    };
    let json = serde_json::to_string(&wire).map_err(|_| JournalError::InvalidFrame)?;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| JournalError::InvalidFrame)?;
    if canonical.as_bytes().len() > MAX_FRAME_BYTES {
        return Err(JournalError::Capacity);
    }
    let payload = objects.iter().try_fold(0usize, |total, object| {
        total
            .checked_add(object.canonical.as_bytes().len())
            .ok_or(JournalError::Capacity)
    })?;
    let envelope = canonical
        .as_bytes()
        .len()
        .checked_sub(payload)
        .ok_or(JournalError::InvalidFrame)?;
    if envelope > MAX_FRAME_NON_PAYLOAD_ENVELOPE {
        return Err(JournalError::Capacity);
    }
    Ok(canonical)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameWire {
    domain: String,
    run_id: RunId,
    run_sequence: u64,
    previous_head_digest: Option<ContentDigest>,
    record: Record,
    objects: Vec<ObjectWire>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectWire {
    content_ref: ContentRef,
    canonical: Box<RawValue>,
}

fn qualify_frame(bytes: &[u8]) -> std::result::Result<QualifiedFrame, JournalError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(JournalError::InvalidHistory);
    }
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|_| JournalError::InvalidHistory)?;
    let wire: FrameWire =
        serde_json::from_slice(canonical.as_bytes()).map_err(|_| JournalError::InvalidHistory)?;
    let FrameWire {
        domain,
        run_id,
        run_sequence,
        previous_head_digest,
        record,
        objects: wire_objects,
    } = wire;
    if domain != "mfm.run.frame.v2"
        || run_sequence == 0
        || run_sequence > MAX_RUN_FRAMES
        || (run_sequence == 1) != previous_head_digest.is_none()
    {
        return Err(JournalError::InvalidHistory);
    }
    if previous_head_digest
        .as_ref()
        .is_some_and(|digest| digest.algorithm() != DigestAlgorithm::Sha256V1)
    {
        return Err(JournalError::InvalidHistory);
    }
    let mut objects = Vec::with_capacity(wire_objects.len());
    for object in wire_objects {
        if objects
            .last()
            .is_some_and(|previous: &ObjectOwned| previous.content_ref >= object.content_ref)
        {
            return Err(JournalError::InvalidHistory);
        }
        let canonical_text = object.canonical.get();
        if canonical_text.len() > MAX_RUN_OBJECT_CANONICAL_BYTES {
            return Err(JournalError::InvalidHistory);
        }
        let object_canonical =
            PlainCanonicalJsonBytes::from_canonical_json_slice(canonical_text.as_bytes())
                .map_err(|_| JournalError::InvalidHistory)?;
        if object.content_ref.content_digest() != &raw_content_digest(object_canonical.as_bytes()) {
            return Err(JournalError::InvalidHistory);
        }
        objects.push(ObjectOwned {
            content_ref: object.content_ref,
            canonical: object_canonical,
        });
    }
    validate_closure(&record, &objects).map_err(|_| JournalError::InvalidHistory)?;
    let reencoded = encode_frame(
        &run_id,
        run_sequence,
        previous_head_digest.as_ref(),
        &record,
        &objects,
    )
    .map_err(|_| JournalError::InvalidHistory)?;
    if reencoded.as_bytes() != bytes {
        return Err(JournalError::InvalidHistory);
    }
    let head_digest = frame_head_digest(bytes);
    Ok(QualifiedFrame {
        run_id,
        run_sequence,
        previous_head_digest,
        record,
        objects,
        canonical,
        head_digest,
    })
}

fn records_are_adjacent(previous: &Record, next: &Record) -> bool {
    match previous {
        Record::StateEffectPrepared { .. } => {
            matches!(next, Record::StateEffectConcluded { .. })
        }
        _ => !matches!(next, Record::StateEffectConcluded { .. }),
    }
}

#[cfg(test)]
mod tests {
    use mfm_ids::{DigestBytes, SchemaId};

    use super::*;

    fn object_ref(schema_name: &str, bytes: &[u8]) -> ContentRef {
        ContentRef::new(
            SchemaId::new(
                schema_name,
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0; 32]),
            )
            .expect("schema"),
            raw_content_digest(bytes),
        )
        .expect("reference")
    }

    fn maximum_object(suffix: char) -> String {
        let mut text = String::with_capacity(MAX_RUN_OBJECT_CANONICAL_BYTES);
        text.push('"');
        text.extend(std::iter::repeat_n('a', MAX_RUN_OBJECT_CANONICAL_BYTES - 3));
        text.push(suffix);
        text.push('"');
        assert_eq!(text.len(), MAX_RUN_OBJECT_CANONICAL_BYTES);
        text
    }

    #[test]
    fn maximum_sequence_and_three_maximum_objects_fit_before_successor_capacity() {
        let intent = maximum_object('i');
        let evidence = maximum_object('e');
        let outcome = maximum_object('o');
        let maximum_schema_name = format!("m{}", "a".repeat(423));
        let intent_ref = object_ref(&maximum_schema_name, intent.as_bytes());
        let evidence_ref = object_ref(&maximum_schema_name, evidence.as_bytes());
        let outcome_ref = object_ref(&maximum_schema_name, outcome.as_bytes());
        assert_eq!(intent_ref.schema_id().as_str().len(), 512);
        assert_eq!(evidence_ref.schema_id().as_str().len(), 512);
        assert_eq!(outcome_ref.schema_id().as_str().len(), 512);
        assert_ne!(intent_ref, evidence_ref);
        assert_ne!(intent_ref, outcome_ref);
        assert_ne!(evidence_ref, outcome_ref);

        let frame = construct_frame(
            RunId::from_digest(DigestBytes::from_array([14; 32])),
            MAX_RUN_FRAMES,
            Some(ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                DigestBytes::from_array([7; 32]),
            )),
            Record::StateConcludedRead {
                intent: intent_ref.clone(),
                evidence: evidence_ref.clone(),
                outcome: Outcome::new(OutcomeKind::Success, outcome_ref.clone()),
            },
            vec![
                (outcome_ref, outcome.as_bytes()),
                (intent_ref, intent.as_bytes()),
                (evidence_ref, evidence.as_bytes()),
            ],
        )
        .expect("maximum frame");
        assert_eq!(frame.run_sequence(), MAX_RUN_FRAMES);
        assert!(frame.canonical_bytes().len() <= MAX_FRAME_BYTES);
        let payload = intent.len() + evidence.len() + outcome.len();
        let envelope = frame
            .canonical_bytes()
            .len()
            .checked_sub(payload)
            .expect("payload is contained in frame");
        assert!(envelope <= MAX_FRAME_NON_PAYLOAD_ENVELOPE);

        let total_bytes = u64::try_from(frame.canonical_bytes().len()).expect("frame length");
        let history = JournalHistory {
            frames: vec![frame.frame],
            total_bytes,
        };
        let next = b"null";
        assert!(matches!(
            history.encode_pure_conclusion(
                OutcomeKind::Success,
                &object_ref("mfm.test.next", next),
                next,
            ),
            Err(JournalError::Capacity)
        ));
    }

    #[test]
    fn transfer_capacity_boundaries_are_exact_without_large_allocations() {
        assert_eq!(
            validate_transfer_lengths(0, std::iter::empty()),
            Err(JournalError::InvalidHistory)
        );
        assert_eq!(
            validate_transfer_lengths(MAX_RUN_FRAMES as usize + 1, std::iter::empty()),
            Err(JournalError::Capacity)
        );
        assert_eq!(
            validate_transfer_lengths(
                MAX_RUN_FRAMES as usize,
                std::iter::repeat_n(0, MAX_RUN_FRAMES as usize),
            ),
            Ok(())
        );
        assert_eq!(validate_transfer_lengths(1, [MAX_FRAME_BYTES]), Ok(()));
        assert_eq!(
            validate_transfer_lengths(1, [MAX_FRAME_BYTES + 1]),
            Err(JournalError::Capacity)
        );

        let full_frames = (MAX_RUN_BYTES / MAX_FRAME_BYTES as u64) as usize;
        let remainder = (MAX_RUN_BYTES % MAX_FRAME_BYTES as u64) as usize;
        let exact =
            std::iter::repeat_n(MAX_FRAME_BYTES, full_frames).chain(std::iter::once(remainder));
        assert_eq!(validate_transfer_lengths(full_frames + 1, exact), Ok(()));
        let overflow =
            std::iter::repeat_n(MAX_FRAME_BYTES, full_frames).chain(std::iter::once(remainder + 1));
        assert_eq!(
            validate_transfer_lengths(full_frames + 1, overflow),
            Err(JournalError::Capacity)
        );
    }
}
