use std::pin::Pin;

use mfm_canonical::{
    CanonicalBytes, CanonicalValue, PlainCanonicalJsonBytes, RecoverabilityContract,
    ValidatedCanonicalValue,
};
use mfm_ids::{ContentRef, EntryPointId, InvocationIdentity, RunId, SchemaId, StableId};
use mfm_journal::structured::JournalHead;
pub use mfm_replay::portable::{ExportKind, PORTABLE_RUN_EXPORT_MEDIA_TYPE};
pub use mfm_replay::structured::{
    StructuredAccessAuditEntry as AccessAuditEntry, StructuredReplayResult as ReplayResponse,
    StructuredTransitionTrace as CanonicalTransitionTrace,
};
pub use mfm_spec::{EntryPointContract, PlanningProfile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncRead;

use crate::{ErrorClass, PublicError};

const ADMIT_RUN_REQUEST_CONTRACT: &str = "mfm.admit-run-request.v1";
const ADMIT_RUN_RESPONSE_CONTRACT: &str = "mfm.admit-run-response.v1";
const DRIVE_RESPONSE_CONTRACT: &str = "mfm.drive-response.v1";
const PUBLIC_RUN_VIEW_CONTRACT: &str = "mfm.public-run-view.v1";
const REPLAY_MODE_CONTRACT: &str = "mfm.replay-mode.v1";
const INSPECTION_CURSOR_PREFIX: &str = "mfm.inspection-cursor.v1.";
const MAX_CURSOR_BYTES: usize = 4_096;
const MAX_CURSOR_ENCODED_BYTES: usize = (MAX_CURSOR_BYTES * 4).div_ceil(3);

/// Frozen admission request version.
pub const ADMIT_RUN_REQUEST_VERSION: &str = "mfm.admit-run-request.v1";
/// Default number of entries returned by trace and audit inspection.
pub const DEFAULT_PAGE_LIMIT: u16 = 100;
/// Maximum number of entries returned by trace and audit inspection.
pub const MAX_PAGE_LIMIT: u16 = 500;

fn recoverability_contract() -> Result<&'static RecoverabilityContract, PublicError> {
    RecoverabilityContract::embedded().map_err(|_| {
        PublicError::internal(
            "RecoverabilityContractUnavailable",
            "The recoverability contract is unavailable",
        )
    })
}

fn invalid_request(code: &'static str, message: &'static str) -> PublicError {
    PublicError::bad_request(code, message)
}

fn canonicalize_transport_json(
    bytes: &[u8],
    code: &'static str,
    message: &'static str,
) -> Result<PlainCanonicalJsonBytes, PublicError> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid_request(code, message))?;
    PlainCanonicalJsonBytes::from_json_str(text).map_err(|_| invalid_request(code, message))
}

fn checked_value(
    contract: &'static str,
    value: CanonicalValue,
    code: &'static str,
    message: &'static str,
) -> Result<ValidatedCanonicalValue, PublicError> {
    recoverability_contract()?
        .encode(contract, &value)
        .map_err(|_| invalid_request(code, message))
}

fn canonical_object(
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue, PublicError> {
    CanonicalValue::object(entries).map_err(|_| {
        PublicError::internal(
            "CanonicalResponseConstructionFailed",
            "A canonical response could not be constructed",
        )
    })
}

fn canonical_content_ref(value: &ContentRef) -> Result<CanonicalValue, PublicError> {
    canonical_object([
        (
            "schema_id",
            CanonicalValue::String(value.schema_id().as_str().to_owned()),
        ),
        (
            "content_digest",
            CanonicalValue::String(value.content_digest().as_str().to_owned()),
        ),
    ])
}

/// Shared transport request for one fixed-head trace or audit page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageRequest {
    cursor: Option<String>,
    limit: Option<u16>,
}

impl PageRequest {
    /// Constructs one bounded page request.
    pub fn new(cursor: Option<String>, limit: Option<u16>) -> Result<Self, PublicError> {
        if limit.is_some_and(|limit| limit == 0 || limit > MAX_PAGE_LIMIT) {
            return Err(page_request_invalid());
        }
        Ok(Self { cursor, limit })
    }

    /// Returns the opaque continuation cursor.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Returns the requested limit, defaulting to 100.
    pub const fn effective_limit(&self) -> u16 {
        match self.limit {
            Some(limit) => limit,
            None => DEFAULT_PAGE_LIMIT,
        }
    }
}

/// Scalar inspection-page position decoded from an app-owned opaque cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InspectionPagePosition {
    /// Physical head fixed by a continuation cursor, or `None` for the first page.
    pub(crate) complete_as_of_journal_head: Option<JournalHead>,
    /// Zero-based authorization index at which the store projection starts.
    pub(crate) start: u32,
    /// Bounded number of entries requested from the store projection.
    pub(crate) limit: u16,
}

/// Decodes one public audit page request into store-facing scalar inputs.
pub(crate) fn decode_access_audit_page_request(
    run_id: &RunId,
    request: &PageRequest,
) -> Result<InspectionPagePosition, PublicError> {
    decode_inspection_page_request(run_id, request, InspectionPurpose::Audit)
}

/// Decodes one public trace page request into store-facing scalar inputs.
pub(crate) fn decode_transition_trace_page_request(
    run_id: &RunId,
    request: &PageRequest,
) -> Result<InspectionPagePosition, PublicError> {
    decode_inspection_page_request(run_id, request, InspectionPurpose::Trace)
}

fn decode_inspection_page_request(
    run_id: &RunId,
    request: &PageRequest,
    purpose: InspectionPurpose,
) -> Result<InspectionPagePosition, PublicError> {
    let Some(cursor) = request.cursor() else {
        return Ok(InspectionPagePosition {
            complete_as_of_journal_head: None,
            start: 0,
            limit: request.effective_limit(),
        });
    };
    let decoded = decode_inspection_cursor(cursor, purpose, run_id)?;
    Ok(InspectionPagePosition {
        complete_as_of_journal_head: Some(decoded.complete_as_of_journal_head),
        start: decoded.next_index,
        limit: request.effective_limit(),
    })
}

/// One head-fixed transition-trace page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionTracePage {
    run_id: RunId,
    at_journal_head: JournalHead,
    transitions: Vec<CanonicalTransitionTrace>,
    next_cursor: Option<String>,
}

impl TransitionTracePage {
    /// Returns the inspected run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the physical head fixed by the first page.
    pub const fn at_journal_head(&self) -> &JournalHead {
        &self.at_journal_head
    }

    /// Returns transition traces in committed order.
    pub fn transitions(&self) -> &[CanonicalTransitionTrace] {
        &self.transitions
    }

    /// Returns the next opaque cursor, if another page exists.
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }
}

/// Wraps a replay-owned scalar continuation in the app-owned opaque public cursor.
pub(crate) fn complete_transition_trace_page(
    page: mfm_replay::structured::StructuredProjectionPage<CanonicalTransitionTrace>,
) -> Result<TransitionTracePage, PublicError> {
    let (run_id, at_journal_head, transitions, next_index) = page.into_parts();
    let next_cursor = match next_index {
        None => None,
        Some(next_index) => Some(encode_inspection_cursor(
            InspectionPurpose::Trace,
            &run_id,
            &at_journal_head,
            next_index,
        )?),
    };
    Ok(TransitionTracePage {
        run_id,
        at_journal_head,
        transitions,
        next_cursor,
    })
}

/// One head-fixed safe external-access audit page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessAuditPage {
    run_id: RunId,
    complete_as_of_journal_head: JournalHead,
    entries: Vec<AccessAuditEntry>,
    next_cursor: Option<String>,
}

impl AccessAuditPage {
    /// Returns the inspected run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the complete physical head fixed by the first page.
    pub const fn complete_as_of_journal_head(&self) -> &JournalHead {
        &self.complete_as_of_journal_head
    }

    /// Returns safe entries in authorization order.
    pub fn entries(&self) -> &[AccessAuditEntry] {
        &self.entries
    }

    /// Returns the next opaque cursor, if another page exists.
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }
}

/// Wraps a replay-owned scalar continuation in the app-owned opaque public cursor.
pub(crate) fn complete_access_audit_page(
    page: mfm_replay::structured::StructuredProjectionPage<AccessAuditEntry>,
) -> Result<AccessAuditPage, PublicError> {
    let (run_id, complete_as_of_journal_head, entries, next_index) = page.into_parts();
    let next_cursor = match next_index {
        None => None,
        Some(next_index) => Some(encode_inspection_cursor(
            InspectionPurpose::Audit,
            &run_id,
            &complete_as_of_journal_head,
            next_index,
        )?),
    };
    Ok(AccessAuditPage {
        run_id,
        complete_as_of_journal_head,
        entries,
        next_cursor,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InspectionPurpose {
    Trace,
    Audit,
}

impl InspectionPurpose {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Audit => "audit",
        }
    }
}

struct InspectionCursor {
    complete_as_of_journal_head: JournalHead,
    next_index: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectionCursorWire {
    version: String,
    purpose: String,
    run_id: RunId,
    at_journal_head: JournalHead,
    next_index: u32,
}

fn encode_inspection_cursor(
    purpose: InspectionPurpose,
    run_id: &RunId,
    complete_as_of_journal_head: &JournalHead,
    next_index: u32,
) -> Result<String, PublicError> {
    let cursor = mfm_journal::structured::canonical_json(&InspectionCursorWire {
        version: "mfm.inspection-cursor.v1".to_owned(),
        purpose: purpose.as_str().to_owned(),
        run_id: run_id.clone(),
        at_journal_head: complete_as_of_journal_head.clone(),
        next_index,
    })
    .map_err(|_| page_cursor_encoding_failed())?;
    if cursor.as_bytes().len() > MAX_CURSOR_BYTES {
        return Err(page_cursor_encoding_failed());
    }
    Ok(format!(
        "{INSPECTION_CURSOR_PREFIX}{}",
        CanonicalBytes::new(cursor.as_bytes().to_vec()).encoded()
    ))
}

fn decode_inspection_cursor(
    encoded: &str,
    expected_purpose: InspectionPurpose,
    expected_run_id: &RunId,
) -> Result<InspectionCursor, PublicError> {
    let encoded = encoded
        .strip_prefix(INSPECTION_CURSOR_PREFIX)
        .ok_or_else(page_request_invalid)?;
    if encoded.len() > MAX_CURSOR_ENCODED_BYTES {
        return Err(page_request_invalid());
    }
    let bytes = CanonicalBytes::from_base64url_no_pad(encoded.to_owned())
        .map_err(|_| page_request_invalid())?;
    if bytes.as_bytes().len() > MAX_CURSOR_BYTES {
        return Err(page_request_invalid());
    }
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes.as_bytes())
        .map_err(|_| page_request_invalid())?;
    let decoded: InspectionCursorWire =
        serde_json::from_slice(canonical.as_bytes()).map_err(|_| page_request_invalid())?;
    if decoded.version != "mfm.inspection-cursor.v1"
        || decoded.purpose != expected_purpose.as_str()
        || &decoded.run_id != expected_run_id
        || decoded.at_journal_head.run_sequence == 0
    {
        return Err(page_request_invalid());
    }
    Ok(InspectionCursor {
        complete_as_of_journal_head: decoded.at_journal_head,
        next_index: decoded.next_index,
    })
}

fn page_request_invalid() -> PublicError {
    PublicError::bad_request(
        "PageRequestInvalid",
        "The page request or cursor is invalid",
    )
}

fn page_cursor_encoding_failed() -> PublicError {
    PublicError::internal(
        "PageCursorEncodingFailed",
        "A page cursor could not be encoded",
    )
}

/// Strict, annex-validated admission request.
///
/// This type has no Serde implementation. Transports must pass their JSON bytes through
/// [`Self::decode_json`], which rejects duplicate keys, floats, wrong literals, unknown fields,
/// and invalid checked identities before authorization.
#[derive(Clone)]
pub struct AdmitRunRequest {
    validated: ValidatedCanonicalValue,
    entry_point_id: EntryPointId,
    invocation_identity: InvocationIdentity,
    input: mfm_spec::CanonicalJsonValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmitRunRequestWire {
    version: String,
    entry_point_id: EntryPointId,
    invocation_identity: InvocationIdentity,
    input: Value,
}

impl AdmitRunRequest {
    /// Decodes ordinary transport JSON and mints one annex-validated request.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, PublicError> {
        let canonical = canonicalize_transport_json(
            bytes,
            "AdmissionRequestInvalid",
            "Admission request JSON is invalid",
        )?;
        let validated = recoverability_contract()?
            .strict_decode(ADMIT_RUN_REQUEST_CONTRACT, canonical.as_bytes())
            .map_err(|_| {
                invalid_request(
                    "AdmissionRequestInvalid",
                    "Admission request does not match the frozen contract",
                )
            })?;
        let wire: AdmitRunRequestWire =
            serde_json::from_slice(validated.as_bytes()).map_err(|_| {
                invalid_request(
                    "AdmissionRequestInvalid",
                    "Admission request does not match the frozen contract",
                )
            })?;
        if wire.version != ADMIT_RUN_REQUEST_VERSION {
            return Err(invalid_request(
                "AdmissionRequestInvalid",
                "Admission request does not match the frozen contract",
            ));
        }
        let input = mfm_spec::CanonicalJsonValue::new(wire.input).map_err(|_| {
            invalid_request(
                "AdmissionRequestInvalid",
                "Admission request input is not canonical JSON",
            )
        })?;
        Ok(Self {
            validated,
            entry_point_id: wire.entry_point_id,
            invocation_identity: wire.invocation_identity,
            input,
        })
    }

    /// Constructs a request from already typed transport fields.
    pub fn new(
        entry_point_id: EntryPointId,
        invocation_identity: InvocationIdentity,
        input: mfm_spec::CanonicalJsonValue,
    ) -> Result<Self, PublicError> {
        let input_bytes = input.canonical_json().map_err(|_| {
            invalid_request(
                "AdmissionRequestInvalid",
                "Admission request input is not canonical JSON",
            )
        })?;
        let input_value = recoverability_contract()?
            .strict_decode("mfm.primitive-canonical_value.v1", input_bytes.as_bytes())
            .and_then(|value| value.canonical_value())
            .map_err(|_| {
                invalid_request(
                    "AdmissionRequestInvalid",
                    "Admission request input is not canonical JSON",
                )
            })?;
        let value = canonical_object([
            (
                "version",
                CanonicalValue::String(ADMIT_RUN_REQUEST_VERSION.to_owned()),
            ),
            (
                "entry_point_id",
                CanonicalValue::String(entry_point_id.as_str().to_owned()),
            ),
            (
                "invocation_identity",
                CanonicalValue::String(invocation_identity.as_str().to_owned()),
            ),
            ("input", input_value),
        ])?;
        let validated = checked_value(
            ADMIT_RUN_REQUEST_CONTRACT,
            value,
            "AdmissionRequestInvalid",
            "Admission request does not match the frozen contract",
        )?;
        Ok(Self {
            validated,
            entry_point_id,
            invocation_identity,
            input,
        })
    }

    /// Returns the exact published entry-point id.
    pub const fn entry_point_id(&self) -> &EntryPointId {
        &self.entry_point_id
    }

    /// Returns the canonical caller invocation identity.
    pub const fn invocation_identity(&self) -> &InvocationIdentity {
        &self.invocation_identity
    }

    /// Returns the checked entry-point input.
    pub const fn input(&self) -> &mfm_spec::CanonicalJsonValue {
        &self.input
    }

    /// Returns the exact canonical request bytes admitted by the annex.
    pub fn as_bytes(&self) -> &[u8] {
        self.validated.as_bytes()
    }
}

impl std::fmt::Debug for AdmitRunRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmitRunRequest")
            .field("entry_point_id", &self.entry_point_id)
            .field("invocation_identity", &self.invocation_identity)
            .finish_non_exhaustive()
    }
}

/// Observable admission result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionStatus {
    /// This call durably admitted the root.
    NewlyAdmitted,
    /// The exact root was already committed.
    Attached,
    /// The commit acknowledgement was ambiguous.
    OutcomeUnknown,
}

impl AdmissionStatus {
    /// Returns the frozen wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NewlyAdmitted => "newly_admitted",
            Self::Attached => "attached",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }
}

macro_rules! canonical_response {
    ($name:ident, $contract:expr, $description:literal, canonical_value) => {
        canonical_response!(@base $name, $contract, $description);

        impl $name {
            pub(crate) fn from_canonical_value(value: CanonicalValue) -> Result<Self, PublicError> {
                let canonical = mfm_canonical::CanonicalJsonBytes::from_value(&value);
                Self::strict_decode(canonical.as_bytes())
            }
        }
    };
    ($name:ident, $contract:expr, $description:literal, serializable) => {
        canonical_response!(@base $name, $contract, $description);

        impl $name {
            pub(crate) fn from_serializable(value: &impl Serialize) -> Result<Self, PublicError> {
                let canonical = mfm_journal::structured::canonical_json(value).map_err(|_| {
                    PublicError::internal(
                        "CanonicalResponseConstructionFailed",
                        "A canonical response could not be constructed",
                    )
                })?;
                Self::strict_decode(canonical.as_bytes())
            }
        }
    };
    (@base $name:ident, $contract:expr, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, PartialEq, Eq)]
        pub struct $name {
            canonical: PlainCanonicalJsonBytes,
            schema_id: SchemaId,
        }

        impl $name {
            /// Strictly decodes exact canonical float-free response bytes.
            pub fn strict_decode(bytes: &[u8]) -> Result<Self, PublicError> {
                let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
                    .map_err(|_| {
                        PublicError::backend(
                            ErrorClass::Internal,
                            "CanonicalResponseInvalid",
                            "A canonical application response failed validation",
                        )
                    })?;
                let validated = recoverability_contract()?
                    .strict_decode($contract, canonical.as_bytes())
                    .map_err(|_| {
                        PublicError::backend(
                            ErrorClass::Internal,
                            "CanonicalResponseInvalid",
                            "A canonical application response failed validation",
                        )
                    })?;
                Ok(Self {
                    canonical,
                    schema_id: validated.schema_id().clone(),
                })
            }

            /// Returns exact canonical response bytes.
            pub fn as_bytes(&self) -> &[u8] {
                self.canonical.as_bytes()
            }

            /// Returns the current response schema identity.
            pub const fn schema_id(&self) -> &SchemaId {
                &self.schema_id
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("schema_id", self.schema_id())
                    .finish_non_exhaustive()
            }
        }
    };
}

canonical_response!(
    AdmitRunResponse,
    ADMIT_RUN_RESPONSE_CONTRACT,
    "Exact annex-validated admission response.",
    canonical_value
);
canonical_response!(
    DriveResponse,
    DRIVE_RESPONSE_CONTRACT,
    "Exact annex-validated result of one run action.",
    serializable
);
canonical_response!(
    PublicRunView,
    PUBLIC_RUN_VIEW_CONTRACT,
    "Exact annex-validated ordinary public run view.",
    serializable
);

impl PublicRunView {
    pub(crate) fn from_structured(
        verified: &mfm_store::structured::VerifiedStructuredRun,
    ) -> Result<Self, PublicError> {
        let outcome = mfm_replay::structured::project_operation_outcome(verified)
            .map_err(|_| PublicError::replay_verification_failed())?
            .map(|outcome| {
                let value: serde_json::Value =
                    serde_json::from_slice(outcome.canonical_value().as_bytes())
                        .map_err(|_| PublicError::replay_verification_failed())?;
                Ok::<_, PublicError>(serde_json::json!({
                    "kind": outcome.kind(),
                    "value_ref": outcome.value(),
                    "value": value,
                }))
            })
            .transpose()?;
        Self::from_serializable(&serde_json::json!({
            "version": PUBLIC_RUN_VIEW_CONTRACT,
            "run_id": verified.run_id(),
            "tenant_scope_id": verified.admission().tenant_scope_id,
            "invocation_identity": verified.admission().invocation_identity,
            "entry_point_operation_id": verified.admission().entry_point_operation_id,
            "journal_head": verified.journal_head(),
            "semantic_head": verified.semantic_head(),
            "status": match verified.frontier() {
                mfm_store::structured::StructuredFrontier::Actions(_) => "actionable",
                mfm_store::structured::StructuredFrontier::WaitingReads => "waiting_reads",
                mfm_store::structured::StructuredFrontier::PossibleEntry => "possible_entry",
                mfm_store::structured::StructuredFrontier::BlockedIntegrity => "blocked_integrity",
                mfm_store::structured::StructuredFrontier::Complete => "closed",
            },
            "outcome": outcome,
        }))
    }
}

impl AdmitRunResponse {
    pub(crate) fn new(
        run_id: &RunId,
        admission: AdmissionStatus,
        entry_point_id: &EntryPointId,
        entry_point_operation_id: &StableId,
        invocation_identity: &InvocationIdentity,
        planning_profile_ref: &ContentRef,
    ) -> Result<Self, PublicError> {
        Self::from_canonical_value(canonical_object([
            (
                "version",
                CanonicalValue::String(ADMIT_RUN_RESPONSE_CONTRACT.to_owned()),
            ),
            ("run_id", CanonicalValue::String(run_id.as_str().to_owned())),
            (
                "admission",
                CanonicalValue::String(admission.as_str().to_owned()),
            ),
            (
                "entry_point_id",
                CanonicalValue::String(entry_point_id.as_str().to_owned()),
            ),
            (
                "entry_point_operation_id",
                CanonicalValue::String(entry_point_operation_id.as_str().to_owned()),
            ),
            (
                "invocation_identity",
                CanonicalValue::String(invocation_identity.as_str().to_owned()),
            ),
            (
                "planning_profile_ref",
                canonical_content_ref(planning_profile_ref)?,
            ),
        ])?)
    }

    /// Returns the admission observation used by transport status mapping.
    pub fn admission(&self) -> Result<AdmissionStatus, PublicError> {
        #[derive(Deserialize)]
        struct Wire {
            admission: String,
        }
        let wire: Wire = serde_json::from_slice(self.as_bytes()).map_err(|_| {
            PublicError::internal(
                "CanonicalResponseProjectionFailed",
                "A canonical application response could not be projected",
            )
        })?;
        match wire.admission.as_str() {
            "newly_admitted" => Ok(AdmissionStatus::NewlyAdmitted),
            "attached" => Ok(AdmissionStatus::Attached),
            "outcome_unknown" => Ok(AdmissionStatus::OutcomeUnknown),
            _ => Err(PublicError::internal(
                "CanonicalResponseProjectionFailed",
                "A canonical application response could not be projected",
            )),
        }
    }
}

impl DriveResponse {
    pub(crate) fn from_runtime(
        run_id: &RunId,
        outcome: mfm_runtime::structured::DriveOutcome,
        journal_head: &JournalHead,
    ) -> Result<Self, PublicError> {
        let (kind, reason) = match outcome {
            mfm_runtime::structured::DriveOutcome::TransitionCommitted { closed: true }
            | mfm_runtime::structured::DriveOutcome::Closed => ("closed", None),
            mfm_runtime::structured::DriveOutcome::TransitionCommitted { closed: false }
            | mfm_runtime::structured::DriveOutcome::AccessObserved
            | mfm_runtime::structured::DriveOutcome::ConcurrentProgress => ("advanced", None),
            mfm_runtime::structured::DriveOutcome::WaitingReads => {
                ("waiting", Some("retryable_evidence_gap"))
            }
            mfm_runtime::structured::DriveOutcome::PossibleEntry => {
                ("waiting", Some("operational_block"))
            }
            mfm_runtime::structured::DriveOutcome::BlockedIntegrity => {
                ("waiting", Some("integrity_block"))
            }
        };
        Self::from_serializable(&serde_json::json!({
            "kind": kind,
            "run_id": run_id,
            "journal_head": journal_head,
            "reason": reason,
        }))
    }
}

/// Public replay mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    /// Verify recorded history without callbacks.
    Verify,
    /// Reproduce under the exact admitted executable when available.
    Reproduce,
    /// Compare the serving executable without authority upgrade.
    CompareCurrent,
}

impl ReplayMode {
    /// Parses the frozen transport spelling and validates it through the annex.
    pub fn parse(value: &str) -> Result<Self, PublicError> {
        let mode = match value {
            "verify" => Self::Verify,
            "reproduce" => Self::Reproduce,
            "compare_current" => Self::CompareCurrent,
            _ => {
                return Err(invalid_request(
                    "ReplayModeInvalid",
                    "Replay mode must be verify, reproduce, or compare_current",
                ))
            }
        };
        recoverability_contract()?
            .encode(
                REPLAY_MODE_CONTRACT,
                &CanonicalValue::String(mode.as_annex_str().to_owned()),
            )
            .map_err(|_| {
                invalid_request(
                    "ReplayModeInvalid",
                    "Replay mode does not match the frozen contract",
                )
            })?;
        Ok(mode)
    }

    /// Returns the annex spelling.
    pub const fn as_annex_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Reproduce => "reproduce",
            Self::CompareCurrent => "compare_current",
        }
    }
}

/// Affine asynchronous reader used for portable export streams.
pub type ExportAsyncReader = Pin<Box<dyn AsyncRead + Send + Unpin + 'static>>;

/// Caller-held semantic portable export stream used by non-verify replay modes.
///
/// Construction does not poll the reader. Full framing, digest, store, tenant, run, head, and
/// closure validation happens only after separate replay and same-run export authorization.
pub struct ExportStreamInput {
    content_ref: ContentRef,
    reader: ExportAsyncReader,
}

impl ExportStreamInput {
    /// Binds one exact content reference to an unpolled caller-held stream.
    pub fn from_reader(
        content_ref: ContentRef,
        reader: ExportAsyncReader,
    ) -> Result<Self, PublicError> {
        let expected_schema = recoverability_contract()?
            .schema_id("mfm.portable-run-export-stream.v1")
            .map_err(|_| {
                PublicError::internal(
                    "RecoverabilityContractUnavailable",
                    "The recoverability contract is unavailable",
                )
            })?;
        if content_ref.schema_id() != expected_schema {
            return Err(PublicError::replay_artifact_invalid());
        }
        Ok(Self {
            content_ref,
            reader,
        })
    }

    /// Returns the caller-supplied exact export identity.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }

    pub(crate) fn into_parts(self) -> (ContentRef, ExportAsyncReader) {
        (self.content_ref, self.reader)
    }
}

impl std::fmt::Debug for ExportStreamInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExportStreamInput")
            .field("content_ref", &self.content_ref)
            .finish_non_exhaustive()
    }
}

/// Checked replay request.
#[derive(Debug)]
pub enum ReplayRequest {
    /// Verify recorded history without any supplied export.
    Verify,
    /// Validate caller-held semantic export evidence before exact reproduction.
    Reproduce(ExportStreamInput),
    /// Validate caller-held semantic export evidence before current-candidate comparison.
    CompareCurrent(ExportStreamInput),
}

impl ReplayRequest {
    /// Returns the requested replay mode.
    pub const fn mode(&self) -> ReplayMode {
        match self {
            Self::Verify => ReplayMode::Verify,
            Self::Reproduce(_) => ReplayMode::Reproduce,
            Self::CompareCurrent(_) => ReplayMode::CompareCurrent,
        }
    }

    pub(crate) const fn requires_export_authorization(&self) -> bool {
        !matches!(self, Self::Verify)
    }
}

/// Checked portable-export request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportRequest {
    kind: ExportKind,
}

impl ExportRequest {
    /// Constructs one checked portable-export request.
    pub const fn new(kind: ExportKind) -> Self {
        Self { kind }
    }

    /// Returns the requested export scope.
    pub const fn kind(self) -> ExportKind {
        self.kind
    }
}

/// App-owned metadata and affine reader for one complete portable export stream.
///
/// The reader is backed by private application storage and cannot be constructed from arbitrary
/// response bytes.
pub struct ExportedRun {
    content_ref: ContentRef,
    reader: ExportAsyncReader,
}

impl ExportedRun {
    pub(crate) const fn from_content_ref(
        content_ref: ContentRef,
        reader: ExportAsyncReader,
    ) -> Self {
        Self {
            content_ref,
            reader,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) const fn for_test(content_ref: ContentRef, reader: ExportAsyncReader) -> Self {
        Self {
            content_ref,
            reader,
        }
    }

    /// Returns the exact response media type.
    pub fn media_type(&self) -> &'static str {
        PORTABLE_RUN_EXPORT_MEDIA_TYPE
    }

    /// Returns the external digest over every exact stream byte.
    pub const fn digest(&self) -> &mfm_ids::ContentDigest {
        self.content_ref.content_digest()
    }

    /// Returns the annex-derived stream schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        self.content_ref.schema_id()
    }

    /// Returns the exact interpretation-and-byte identity written beside a CLI export.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }

    /// Moves out the complete export stream reader.
    pub fn into_reader(self) -> ExportAsyncReader {
        self.reader
    }
}

impl std::fmt::Debug for ExportedRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExportedRun")
            .field("content_ref", self.content_ref())
            .field("media_type", &self.media_type())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use mfm_ids::JournalCommitDigest;
    use mfm_journal::structured::JournalHead;
    use serde::Deserialize;
    use static_assertions::assert_not_impl_any;

    use super::{
        decode_access_audit_page_request, decode_transition_trace_page_request,
        encode_inspection_cursor, AdmissionStatus, AdmitRunRequest, AdmitRunResponse, ContentRef,
        DriveResponse, ExportStreamInput, ExportedRun, InspectionPurpose, PageRequest,
        PublicRunView, RecoverabilityContract, ReplayMode, ReplayRequest, RunId,
        DEFAULT_PAGE_LIMIT, INSPECTION_CURSOR_PREFIX, MAX_CURSOR_ENCODED_BYTES, MAX_PAGE_LIMIT,
    };

    assert_not_impl_any!(ExportStreamInput: Clone, Copy);
    assert_not_impl_any!(ExportedRun: Clone, Copy);
    assert_not_impl_any!(ReplayRequest: Clone, Copy);

    #[derive(Deserialize)]
    struct Corpus {
        positive_vectors: Vec<Vector>,
    }

    #[derive(Deserialize)]
    struct Vector {
        id: String,
        canonical_hex: Option<String>,
    }

    #[test]
    fn app_owned_wire_values_round_trip_the_frozen_corpus_bytes() {
        let corpus: Corpus = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/recoverability/v1/corpus.json"
        )))
        .expect("frozen corpus");

        let request = vector_bytes(&corpus, "schema/mfm.admit-run-request.v1/minimum");
        assert_eq!(
            AdmitRunRequest::decode_json(&request)
                .expect("admission request")
                .as_bytes(),
            request
        );

        let admission = vector_bytes(&corpus, "schema/mfm.admit-run-response.v1/minimum");
        let admission = AdmitRunResponse::strict_decode(&admission).expect("admission response");
        assert_eq!(
            admission.admission().expect("status"),
            AdmissionStatus::Attached
        );
        assert_eq!(
            admission.as_bytes(),
            vector_bytes(&corpus, "schema/mfm.admit-run-response.v1/minimum")
        );

        for id in [
            "schema/mfm.drive-response.v1/minimum",
            "schema/mfm.drive-response.v1/waiting",
            "schema/mfm.drive-response.v1/closed",
        ] {
            let bytes = vector_bytes(&corpus, id);
            assert_eq!(
                DriveResponse::strict_decode(&bytes)
                    .expect("drive response")
                    .as_bytes(),
                bytes
            );
        }

        let public = vector_bytes(&corpus, "schema/mfm.public-run-view.v1/minimum");
        assert_eq!(
            PublicRunView::strict_decode(&public)
                .expect("public run view")
                .as_bytes(),
            public
        );
    }

    #[test]
    fn application_replay_mode_accepts_only_the_annex_spelling() {
        assert_eq!(
            ReplayMode::parse("compare_current").expect("annex mode"),
            ReplayMode::CompareCurrent
        );
        assert!(ReplayMode::parse("compare-current").is_err());
    }

    #[test]
    fn app_owned_audit_cursor_round_trips_fixed_head_and_u32_index() {
        let run_id = RunId::parse(format!("run:sha256-jcs-v1:{}", "1".repeat(64))).expect("run id");
        let other_run_id =
            RunId::parse(format!("run:sha256-jcs-v1:{}", "2".repeat(64))).expect("other run id");
        let commit_digest = JournalCommitDigest::parse(format!("sha256-jcs-v1:{}", "3".repeat(64)))
            .expect("commit digest");
        let head = JournalHead {
            run_sequence: 7,
            commit_digest,
        };

        let first = decode_access_audit_page_request(
            &run_id,
            &PageRequest::new(None, None).expect("first page"),
        )
        .expect("decode first page");
        assert_eq!(first.complete_as_of_journal_head, None);
        assert_eq!(first.start, 0);
        assert_eq!(first.limit, DEFAULT_PAGE_LIMIT);

        let first_trace = decode_transition_trace_page_request(
            &run_id,
            &PageRequest::new(None, Some(1)).expect("first trace page"),
        )
        .expect("decode first trace page");
        assert_eq!(first_trace.complete_as_of_journal_head, None);
        assert_eq!(first_trace.start, 0);
        assert_eq!(first_trace.limit, 1);

        let cursor = encode_inspection_cursor(InspectionPurpose::Audit, &run_id, &head, u32::MAX)
            .expect("encode cursor");
        assert!(cursor.starts_with(INSPECTION_CURSOR_PREFIX));
        let continued = decode_access_audit_page_request(
            &run_id,
            &PageRequest::new(Some(cursor.clone()), Some(MAX_PAGE_LIMIT)).expect("continued page"),
        )
        .expect("decode continued page");
        assert_eq!(continued.complete_as_of_journal_head, Some(head.clone()));
        assert_eq!(continued.start, u32::MAX);
        assert_eq!(continued.limit, MAX_PAGE_LIMIT);

        let trace_cursor = encode_inspection_cursor(InspectionPurpose::Trace, &run_id, &head, 8)
            .expect("encode trace cursor");
        let trace_request =
            PageRequest::new(Some(trace_cursor), None).expect("trace cursor request");
        let trace = decode_transition_trace_page_request(&run_id, &trace_request)
            .expect("decode trace cursor");
        assert_eq!(trace.complete_as_of_journal_head.as_ref(), Some(&head));
        assert_eq!(trace.start, 8);
        assert_eq!(trace.limit, DEFAULT_PAGE_LIMIT);
        assert_eq!(
            decode_access_audit_page_request(&run_id, &trace_request)
                .expect_err("trace cursor must not continue audit"),
            super::page_request_invalid()
        );

        let wrong_run = PageRequest::new(Some(cursor.clone()), None).expect("wrong-run request");
        assert_eq!(
            decode_access_audit_page_request(&other_run_id, &wrong_run)
                .expect_err("cursor must be run-bound"),
            super::page_request_invalid()
        );
        let padded =
            PageRequest::new(Some(format!("{cursor}=")), None).expect("padded cursor request");
        assert_eq!(
            decode_access_audit_page_request(&run_id, &padded)
                .expect_err("padded cursor must fail"),
            super::page_request_invalid()
        );
        let oversized = PageRequest::new(
            Some(format!(
                "{INSPECTION_CURSOR_PREFIX}{}",
                "A".repeat(MAX_CURSOR_ENCODED_BYTES + 1)
            )),
            None,
        )
        .expect("oversized cursor request");
        assert_eq!(
            decode_access_audit_page_request(&run_id, &oversized)
                .expect_err("oversized cursor must fail"),
            super::page_request_invalid()
        );
    }

    #[test]
    fn app_owned_page_request_enforces_the_frozen_u16_limit() {
        assert_eq!(PageRequest::default().effective_limit(), DEFAULT_PAGE_LIMIT);
        assert_eq!(
            PageRequest::new(None, Some(MAX_PAGE_LIMIT))
                .expect("maximum page")
                .effective_limit(),
            MAX_PAGE_LIMIT
        );
        assert!(PageRequest::new(None, Some(0)).is_err());
        assert!(PageRequest::new(None, Some(MAX_PAGE_LIMIT + 1)).is_err());
    }

    #[test]
    fn portable_replay_input_binds_an_unpolled_affine_reader() {
        let contract = RecoverabilityContract::embedded().expect("annex");
        let content_ref = ContentRef::new(
            contract
                .schema_id("mfm.portable-run-export-stream.v1")
                .expect("portable schema")
                .clone(),
            contract.raw_content_digest(b"stream"),
        )
        .expect("content ref");
        let input =
            ExportStreamInput::from_reader(content_ref.clone(), Box::pin(tokio::io::empty()))
                .expect("stream input");
        assert_eq!(input.content_ref(), &content_ref);
        assert_eq!(
            ReplayRequest::CompareCurrent(input).mode(),
            ReplayMode::CompareCurrent
        );

        let wrong_ref = ContentRef::new(
            contract
                .schema_id("mfm.content-ref.v1")
                .expect("other schema")
                .clone(),
            contract.raw_content_digest(b"stream"),
        )
        .expect("wrong content ref");
        let error = ExportStreamInput::from_reader(wrong_ref, Box::pin(tokio::io::empty()))
            .expect_err("wrong stream schema");
        assert_eq!(error.code(), "ReplayArtifactInvalid");

        let input =
            ExportStreamInput::from_reader(content_ref.clone(), Box::pin(tokio::io::empty()))
                .expect("debug input");
        let debug = format!("{input:?}");
        assert!(debug.contains(content_ref.content_digest().as_str()));
        assert!(!debug.contains("reader"));
        assert!(!debug.contains("path"));

        let export = ExportedRun::for_test(content_ref, Box::pin(tokio::io::empty()));
        let debug = format!("{export:?}");
        assert!(!debug.contains("reader"));
        assert!(!debug.contains("path"));
    }

    #[test]
    fn admission_transport_rejects_duplicate_fields() {
        let bytes = br#"{
            "version":"mfm.admit-run-request.v1",
            "entry_point_id":"sample",
            "entry_point_id":"sample",
            "invocation_identity":"00000000-0000-4000-8000-000000000000",
            "input":{"sample":"value"}
        }"#;
        assert!(AdmitRunRequest::decode_json(bytes).is_err());
    }

    #[test]
    fn drive_response_rejects_the_superseded_outcome_discriminator() {
        let corpus: Corpus = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/recoverability/v1/corpus.json"
        )))
        .expect("frozen corpus");
        let canonical = vector_bytes(&corpus, "schema/mfm.drive-response.v1/minimum");
        let value: serde_json::Value =
            serde_json::from_slice(&canonical).expect("drive response JSON");
        let mut object = value.as_object().expect("drive response object").clone();
        let kind = object.remove("kind").expect("drive response kind");
        object.insert("outcome".to_owned(), kind);
        let superseded = serde_json::to_vec(&object).expect("superseded drive response JSON");

        assert!(DriveResponse::strict_decode(&superseded).is_err());
    }

    fn vector_bytes(corpus: &Corpus, id: &str) -> Vec<u8> {
        let encoded = corpus
            .positive_vectors
            .iter()
            .find(|vector| vector.id == id)
            .and_then(|vector| vector.canonical_hex.as_deref())
            .expect("golden vector");
        assert_eq!(encoded.len() % 2, 0);
        encoded
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let text = std::str::from_utf8(pair).expect("hex pair");
                u8::from_str_radix(text, 16).expect("hex byte")
            })
            .collect()
    }
}
