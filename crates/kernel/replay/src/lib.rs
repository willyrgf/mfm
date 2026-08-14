#![warn(missing_docs)]
//! Callback-free retained-prefix qualification and strict portable export.
//!
//! Replay consumes only `RunFrame` bytes and Store-qualified prefixes.  It never receives a
//! Runtime assembly and therefore cannot invoke a State, adapter, signer, or provider.

use mfm_canonical::raw_content_digest;
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_journal::single_trust::{RunFrame, RunRecord};
use mfm_program::ProgramCatalog;
use mfm_store::single_trust::{replay_terminality, QualifiedRun, StoreError};
use serde::{Deserialize, Serialize};

/// The only portable stream identity accepted by the cutover.
pub const PORTABLE_FORMAT: &str = "mfm.portable-run.v5";

/// Redaction-safe replay error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReplayError {
    /// The retained prefix is malformed or not contiguous.
    #[error("replay prefix is invalid")]
    InvalidPrefix,
    /// The input exceeded the portable/replay ceiling.
    #[error("replay capacity bound exceeded")]
    Capacity,
    /// Store qualification failed.
    #[error("replay Store qualification failed")]
    Store,
}

/// Callback-free result of qualifying one complete prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayReport {
    head_sequence: u64,
    terminal: bool,
}

impl ReplayReport {
    /// Returns the qualified head sequence.
    pub const fn head_sequence(&self) -> u64 {
        self.head_sequence
    }

    /// Returns whether the prefix contains a terminal failure marker.
    pub const fn terminal(&self) -> bool {
        self.terminal
    }
}

/// Qualifies a Store-owned prefix without invoking any live callback.
pub fn qualify(run: &QualifiedRun) -> Result<ReplayReport, ReplayError> {
    let frames = run.frames();
    if frames.is_empty() || frames.len() > mfm_journal::single_trust::MAX_RUN_FRAMES {
        return Err(ReplayError::InvalidPrefix);
    }
    let terminal = frames.iter().any(|frame| {
        matches!(
            frame.record(),
            RunRecord::StateConcluded(mfm_journal::single_trust::StateConcluded::Pure {
                outcome: mfm_journal::single_trust::StateOutcome::Failure(_),
                ..
            }) | RunRecord::StateConcluded(mfm_journal::single_trust::StateConcluded::Access {
                outcome: mfm_journal::single_trust::StateOutcome::Failure(_),
                ..
            })
        )
    });
    Ok(ReplayReport {
        head_sequence: run.head_sequence(),
        terminal,
    })
}

/// Qualifies a retained prefix and derives terminality from its exact callback-free Program.
///
/// The structural [`qualify`] helper is useful when only the retained three-family stream is
/// available. Consumers that need semantic terminality must use this function so a nonterminal
/// conclusion is not mistaken for a terminal result. The Program is loaded only from the run's
/// admitted object closure and strictly ingressed under `catalog`.
pub fn qualify_with_retained_program(
    run: &QualifiedRun,
    catalog: &ProgramCatalog,
) -> Result<ReplayReport, ReplayError> {
    let terminal = replay_terminality(run, catalog).map_err(|_| ReplayError::Store)?;
    Ok(ReplayReport {
        head_sequence: run.head_sequence(),
        terminal,
    })
}

/// Strict bounded portable stream for one complete retained run.
#[derive(Debug, PartialEq, Eq)]
pub struct PortableRun {
    frames: Vec<RunFrame>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableEnvelope {
    format: String,
    store_scope_id: mfm_ids::StoreScopeId,
    store_epoch: mfm_ids::StoreEpoch,
    tenant_scope_id: mfm_ids::TenantScopeId,
    run_id: mfm_ids::RunId,
    frames: Vec<RunFrame>,
}

impl PortableRun {
    /// Builds an export from a qualified run.
    pub fn from_run(run: &QualifiedRun) -> Self {
        Self {
            frames: run.frames().to_vec(),
        }
    }

    /// Returns the exported frames in exact append order.
    pub fn frames(&self) -> &[RunFrame] {
        &self.frames
    }

    /// Encodes the stream as one canonical v5 structural envelope.
    pub fn encode(&self) -> Result<Vec<u8>, ReplayError> {
        let first = self.frames.first().ok_or(ReplayError::InvalidPrefix)?;
        let admission = match first.record() {
            RunRecord::RunAdmitted(admission) => admission,
            RunRecord::StatePrepared(_) | RunRecord::StateConcluded(_) => {
                return Err(ReplayError::InvalidPrefix)
            }
        };
        let envelope = PortableEnvelope {
            format: PORTABLE_FORMAT.to_owned(),
            store_scope_id: admission.store_scope_id().clone(),
            store_epoch: admission.store_epoch(),
            tenant_scope_id: admission.tenant_scope_id().clone(),
            run_id: admission.run_id().clone(),
            frames: self.frames.clone(),
        };
        let encoded = mfm_journal::single_trust::canonical_json(&envelope)
            .map_err(|_| ReplayError::InvalidPrefix)?;
        if encoded.as_bytes().len() > mfm_journal::single_trust::MAX_RUN_FRAME_BYTES {
            return Err(ReplayError::Capacity);
        }
        Ok(encoded.as_bytes().to_vec())
    }

    /// Returns the content identity of the exact canonical v5 stream.
    pub fn content_ref(&self) -> Result<ContentRef, ReplayError> {
        let bytes = self.encode()?;
        portable_content_ref(&bytes)
    }

    /// Strictly imports one canonical v5 stream and rejects legacy records.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplayError> {
        decode_inner(bytes)
    }

    /// Verifies the expected stream identity before parsing any retained frame.
    pub fn decode_expected(bytes: &[u8], expected: &ContentRef) -> Result<Self, ReplayError> {
        if portable_content_ref(bytes)? != *expected {
            return Err(ReplayError::InvalidPrefix);
        }
        decode_inner(bytes)
    }

    /// Returns the portable frame count.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Returns whether the portable stream is empty.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

fn decode_inner(bytes: &[u8]) -> Result<PortableRun, ReplayError> {
    if bytes.len() > mfm_journal::single_trust::MAX_RUN_FRAME_BYTES {
        return Err(ReplayError::Capacity);
    }
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|_| ReplayError::InvalidPrefix)?;
    let envelope: PortableEnvelope =
        serde_json::from_slice(canonical.as_bytes()).map_err(|_| ReplayError::InvalidPrefix)?;
    if envelope.format != PORTABLE_FORMAT
        || envelope.frames.is_empty()
        || envelope.frames.len() > mfm_journal::single_trust::MAX_RUN_FRAMES
    {
        return Err(ReplayError::InvalidPrefix);
    }
    let frames = envelope.frames;
    if frames
        .iter()
        .enumerate()
        .any(|(index, frame)| frame.expected_sequence() != index as u64 + 1)
    {
        return Err(ReplayError::InvalidPrefix);
    }
    let (scope, epoch, tenant) = match frames[0].record() {
        RunRecord::RunAdmitted(admission) => (
            admission.store_scope_id().clone(),
            admission.store_epoch(),
            admission.tenant_scope_id().clone(),
        ),
        RunRecord::StatePrepared(_) | RunRecord::StateConcluded(_) => {
            return Err(ReplayError::InvalidPrefix)
        }
    };
    if envelope.store_scope_id != scope
        || envelope.store_epoch != epoch
        || envelope.tenant_scope_id != tenant
        || envelope.run_id != *frames[0].run_id()
    {
        return Err(ReplayError::InvalidPrefix);
    }
    mfm_store::single_trust::validate_prefix(scope, epoch, tenant, frames.clone())
        .map_err(|_| ReplayError::InvalidPrefix)?;
    Ok(PortableRun { frames })
}

fn portable_content_ref(bytes: &[u8]) -> Result<ContentRef, ReplayError> {
    if bytes.len() > mfm_journal::single_trust::MAX_RUN_FRAME_BYTES {
        return Err(ReplayError::Capacity);
    }
    let schema = SchemaId::new(
        "mfm.portable-run",
        "5",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| ReplayError::InvalidPrefix)?;
    ContentRef::new(schema, raw_content_digest(bytes)).map_err(|_| ReplayError::InvalidPrefix)
}

impl From<StoreError> for ReplayError {
    fn from(_: StoreError) -> Self {
        Self::Store
    }
}
