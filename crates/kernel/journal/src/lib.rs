#![warn(missing_docs)]
//! Exact canonical, opaque run frames. Runtime owns the payload contract.

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_ids::{ContentDigest, DigestAlgorithm, RunId};
pub use mfm_values::SizeLimitExceeded;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

/// Maximum complete canonical frame bytes.
pub const MAX_FRAME_BYTES: usize = 134_283_264;
/// Maximum frame bytes outside serialized canonical object payloads.
pub const MAX_FRAME_NON_PAYLOAD_ENVELOPE: usize = 65_536;
/// Maximum frames in one run.
pub const MAX_RUN_FRAMES: u64 = 65_536;
/// Maximum cumulative retained bytes in one run.
pub const MAX_RUN_BYTES: u64 = 536_870_912;

/// Failure to seal or decode an exact frame.
#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum JournalError {
    /// A complete frame exceeded its ceiling.
    #[error("frame {0}")]
    FrameSize(SizeLimitExceeded),
    /// The sequence exceeded its ceiling.
    #[error("frame count {0}")]
    FrameCount(SizeLimitExceeded),
    /// The envelope has invalid canonical syntax or header fields.
    #[error("journal frame is invalid")]
    InvalidFrame,
    /// JSON parsing or encoding failed with its reviewed source.
    #[error("frame JSON processing failed")]
    Json {
        /// Whether the caller was sealing or decoding a frame.
        operation: FrameOperation,
        /// Actual parser/serializer failure.
        #[source]
        source: mfm_canonical::JsonError,
    },
    /// Exact canonical grammar failed with its native source retained.
    #[error("frame canonical processing failed")]
    Canonical {
        /// Whether the caller was sealing or decoding a frame.
        operation: FrameOperation,
        /// Actual canonicalization failure.
        #[source]
        source: mfm_canonical::CanonicalError,
    },
}

/// Operation owning an exact frame encoding/decoding failure.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameOperation {
    /// Sealing a caller's canonical payload.
    Seal,
    /// Decoding retained frame bytes.
    Decode,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<'a> {
    domain: &'a str,
    run_id: RunId,
    run_sequence: u64,
    previous_head_digest: Option<ContentDigest>,
    #[serde(borrow)]
    payload: &'a RawValue,
}

/// A sealed frame whose opaque payload is owned by its caller.
#[derive(Clone)]
pub struct EncodedRunFrame {
    run_id: RunId,
    sequence: u64,
    previous: Option<ContentDigest>,
    head: ContentDigest,
    canonical: PlainCanonicalJsonBytes,
    payload: PlainCanonicalJsonBytes,
}

impl std::fmt::Debug for EncodedRunFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodedRunFrame")
            .field("run_id", &self.run_id)
            .field("sequence", &self.sequence)
            .field("head", &self.head)
            .finish_non_exhaustive()
    }
}

impl EncodedRunFrame {
    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }
    /// Returns the one-based sequence.
    pub const fn run_sequence(&self) -> u64 {
        self.sequence
    }
    /// Returns the previous exact frame digest, absent only at admission.
    pub const fn previous_head_digest(&self) -> Option<&ContentDigest> {
        self.previous.as_ref()
    }
    /// Returns the hash of the complete exact frame bytes.
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head
    }
    /// Returns complete immutable frame bytes.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
    /// Returns the canonical payload without interpreting its fields.
    pub const fn payload(&self) -> &PlainCanonicalJsonBytes {
        &self.payload
    }
}

/// Hashes exact frame bytes without decoding them.
pub fn frame_head_digest(bytes: &[u8]) -> ContentDigest {
    raw_content_digest(bytes)
}

fn check_header(sequence: u64, previous: Option<&ContentDigest>) -> Result<(), JournalError> {
    SizeLimitExceeded::check(sequence, MAX_RUN_FRAMES).map_err(JournalError::FrameCount)?;
    if sequence == 0
        || (sequence == 1) != previous.is_none()
        || previous.is_some_and(|digest| digest.algorithm() != DigestAlgorithm::Sha256V1)
    {
        return Err(JournalError::InvalidFrame);
    }
    Ok(())
}

/// Seals a canonical payload under the one current frame envelope.
pub fn seal_frame(
    run_id: &RunId,
    sequence: u64,
    previous: Option<&ContentDigest>,
    payload: &PlainCanonicalJsonBytes,
) -> Result<EncodedRunFrame, JournalError> {
    check_header(sequence, previous)?;
    let raw = serde_json::from_slice::<&RawValue>(payload.as_bytes()).map_err(|source| {
        JournalError::Json {
            operation: FrameOperation::Seal,
            source: mfm_canonical::JsonError::new(source),
        }
    })?;
    let wire = Envelope {
        domain: "mfm.run.frame.v6",
        run_id: run_id.clone(),
        run_sequence: sequence,
        previous_head_digest: previous.cloned(),
        payload: raw,
    };
    let json = mfm_canonical::to_json_bounded(&wire, MAX_FRAME_BYTES).map_err(|source| {
        JournalError::Canonical {
            operation: FrameOperation::Seal,
            source,
        }
    })?;
    SizeLimitExceeded::check(json.len() as u64, MAX_FRAME_BYTES as u64)
        .map_err(JournalError::FrameSize)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).map_err(|source| {
        JournalError::Canonical {
            operation: FrameOperation::Seal,
            source,
        }
    })?;
    Ok(EncodedRunFrame {
        run_id: run_id.clone(),
        sequence,
        previous: previous.cloned(),
        head: frame_head_digest(canonical.as_bytes()),
        canonical,
        payload: payload.clone(),
    })
}

/// Decodes only the exact envelope, canonical grammar and complete-frame ceiling.
pub fn decode_frame(bytes: &[u8]) -> Result<EncodedRunFrame, JournalError> {
    SizeLimitExceeded::check(bytes.len() as u64, MAX_FRAME_BYTES as u64)
        .map_err(JournalError::FrameSize)?;
    let canonical =
        PlainCanonicalJsonBytes::from_canonical_json_slice(bytes).map_err(|source| {
            JournalError::Canonical {
                operation: FrameOperation::Decode,
                source,
            }
        })?;
    let wire: Envelope<'_> =
        serde_json::from_slice(bytes).map_err(|source| JournalError::Json {
            operation: FrameOperation::Decode,
            source: mfm_canonical::JsonError::new(source),
        })?;
    if wire.domain != "mfm.run.frame.v6" {
        return Err(JournalError::InvalidFrame);
    }
    check_header(wire.run_sequence, wire.previous_head_digest.as_ref())?;
    let payload = PlainCanonicalJsonBytes::from_canonical_json_slice(wire.payload.get().as_bytes())
        .map_err(|source| JournalError::Canonical {
            operation: FrameOperation::Decode,
            source,
        })?;
    Ok(EncodedRunFrame {
        run_id: wire.run_id,
        sequence: wire.run_sequence,
        previous: wire.previous_head_digest,
        head: frame_head_digest(bytes),
        canonical,
        payload,
    })
}
