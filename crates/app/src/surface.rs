use std::pin::Pin;

use mfm_canonical::{sha256_digest_bytes, CanonicalBytes, CanonicalValue, PlainCanonicalJsonBytes};
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, EntryPointId, InvocationIdentity, RunId, SchemaId,
    StableId, TenantScopeId,
};
use mfm_journal::structured::{JournalHead, LexicalValueRef, RecordRef, SemanticHead};
pub use mfm_replay::portable::{ExportKind, PORTABLE_RUN_EXPORT_MEDIA_TYPE};
pub use mfm_replay::structured::{
    StructuredAccessAuditEntry as AccessAuditEntry, StructuredReplayResult as ReplayResponse,
    StructuredTransitionTrace as CanonicalTransitionTrace,
};
use mfm_runtime::history::{EffectEntryAttentionResolution, EffectEntrySubject};
pub use mfm_spec::{PlanningProfile, PublishedEntryPoint};
use mfm_values::PersistedSchema as _;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncRead;

use crate::{ErrorClass, PublicError};

const ADMIT_RUN_RESPONSE_CONTRACT: &str = "mfm.admit-run-response.v1";
const DRIVE_RESPONSE_CONTRACT: &str = "mfm.drive-response.v1";
const PUBLIC_RUN_VIEW_CONTRACT: &str = "mfm.public-run-view.v1";
const INSPECTION_CURSOR_PREFIX: &str = "mfm.inspection-cursor.v1.";
const MAX_CURSOR_BYTES: usize = 4_096;
const MAX_CURSOR_ENCODED_BYTES: usize = (MAX_CURSOR_BYTES * 4).div_ceil(3);

/// Frozen admission request version.
pub const ADMIT_RUN_REQUEST_VERSION: &str = "mfm.admit-run-request.v1";
/// Default number of entries returned by trace and audit inspection.
pub const DEFAULT_PAGE_LIMIT: u16 = 100;
/// Maximum number of entries returned by trace and audit inspection.
pub const MAX_PAGE_LIMIT: u16 = 500;

/// Lifts already-canonical JSON into the typed canonical value tree.
fn canonical_value_from_json(value: &Value) -> Option<CanonicalValue> {
    Some(match value {
        Value::Null => CanonicalValue::Null,
        Value::Bool(value) => CanonicalValue::Bool(*value),
        Value::String(value) => CanonicalValue::String(value.clone()),
        Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                CanonicalValue::Unsigned(value)
            } else {
                CanonicalValue::Signed(number.as_i64()?)
            }
        }
        Value::Array(values) => CanonicalValue::Array(
            values
                .iter()
                .map(canonical_value_from_json)
                .collect::<Option<Vec<_>>>()?,
        ),
        Value::Object(entries) => CanonicalValue::object(
            entries
                .iter()
                .map(|(key, value)| Some((key.clone(), canonical_value_from_json(value)?)))
                .collect::<Option<Vec<_>>>()?,
        )
        .ok()?,
    })
}

fn canonical_response_invalid() -> PublicError {
    PublicError::backend(
        ErrorClass::Internal,
        "CanonicalResponseInvalid",
        "A canonical application response failed validation",
    )
}

/// Decodes one exact owner wire and rejects every alternate spelling.
fn decode_exact_response<T>(bytes: &[u8]) -> Result<(PlainCanonicalJsonBytes, T), PublicError>
where
    T: DeserializeOwned + Serialize,
{
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|_| canonical_response_invalid())?;
    let wire: T =
        serde_json::from_slice(canonical.as_bytes()).map_err(|_| canonical_response_invalid())?;
    let encoded =
        mfm_journal::structured::canonical_json(&wire).map_err(|_| canonical_response_invalid())?;
    if encoded.as_bytes() != canonical.as_bytes() {
        return Err(canonical_response_invalid());
    }
    Ok((canonical, wire))
}

fn encode_exact_response<T: Serialize>(wire: &T) -> Result<PlainCanonicalJsonBytes, PublicError> {
    mfm_journal::structured::canonical_json(wire).map_err(|_| {
        PublicError::internal(
            "CanonicalResponseConstructionFailed",
            "A canonical response could not be constructed",
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
    page: mfm_replay::structured::StructuredTransitionTracePage,
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
    page: mfm_replay::structured::StructuredAccessAuditPage,
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

/// One run in the tenant that currently requires manual Effect-entry attention.
///
/// Every field is re-derived from qualified, reduced history. The entry names
/// the run and its exact blocked occurrence; it grants no authority over that
/// run and carries no capability, request, or observation material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntryAttentionEntry {
    /// Exact run identity.
    run_id: RunId,
    /// Exact head at which attention was re-derived.
    journal_head: JournalHead,
    /// Exact unresolved Effect subject.
    subject: EffectEntrySubject,
    /// Reducer-derived operator resolution.
    resolution: EffectEntryAttentionResolution,
}

impl EffectEntryAttentionEntry {
    pub(crate) fn from_reduced(
        run_id: RunId,
        journal_head: JournalHead,
        subject: EffectEntrySubject,
        resolution: EffectEntryAttentionResolution,
    ) -> Self {
        Self {
            run_id,
            journal_head,
            subject,
            resolution,
        }
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact head at which attention was re-derived.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }

    /// Returns the exact unresolved Effect subject.
    pub const fn subject(&self) -> &EffectEntrySubject {
        &self.subject
    }

    /// Returns the reducer-derived operator resolution.
    pub const fn resolution(&self) -> EffectEntryAttentionResolution {
        self.resolution
    }
}

/// One bounded page of current Effect-entry attention, ordered by run identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntryAttentionPage {
    /// Runs on this page, in strict run order.
    entries: Vec<EffectEntryAttentionEntry>,
    /// Opaque continuation cursor, when another page exists.
    next_cursor: Option<String>,
}

impl EffectEntryAttentionPage {
    pub(crate) fn from_entries(
        entries: Vec<EffectEntryAttentionEntry>,
        next_cursor: Option<String>,
    ) -> Self {
        Self {
            entries,
            next_cursor,
        }
    }

    /// Returns the runs on this page, in strict run order.
    pub fn entries(&self) -> &[EffectEntryAttentionEntry] {
        &self.entries
    }

    /// Returns the opaque continuation cursor, when another page exists.
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }
}

const EFFECT_ENTRY_ATTENTION_CURSOR_PREFIX: &str = "mfm.effect-entry-attention-cursor.v1.";

/// Opaque tenant-bound continuation for one attention sweep.
///
/// It is ordered strictly by run identity and carries nothing else. The codec
/// and literal version are owned by this surface type: the cursor has no
/// `SchemaId` because it is transport state, not retained content.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectEntryAttentionCursorWire {
    tenant_scope_id: TenantScopeId,
    after_run_id: RunId,
}

/// Encodes one attention continuation key.
pub(crate) fn encode_effect_entry_attention_cursor(
    tenant_scope_id: &TenantScopeId,
    after_run_id: &RunId,
) -> Result<String, PublicError> {
    let cursor = mfm_journal::structured::canonical_json(&EffectEntryAttentionCursorWire {
        tenant_scope_id: tenant_scope_id.clone(),
        after_run_id: after_run_id.clone(),
    })
    .map_err(|_| page_cursor_encoding_failed())?;
    if cursor.as_bytes().len() > MAX_CURSOR_BYTES {
        return Err(page_cursor_encoding_failed());
    }
    Ok(format!(
        "{EFFECT_ENTRY_ATTENTION_CURSOR_PREFIX}{}",
        CanonicalBytes::new(cursor.as_bytes().to_vec()).encoded()
    ))
}

/// Strictly decodes one attention continuation key for an exact tenant.
///
/// A cursor minted for another tenant, an unknown prefix, or an unknown field
/// is refused rather than reinterpreted.
pub(crate) fn decode_effect_entry_attention_cursor(
    encoded: &str,
    expected_tenant_scope_id: &TenantScopeId,
) -> Result<RunId, PublicError> {
    let encoded = encoded
        .strip_prefix(EFFECT_ENTRY_ATTENTION_CURSOR_PREFIX)
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
    let decoded: EffectEntryAttentionCursorWire =
        serde_json::from_slice(canonical.as_bytes()).map_err(|_| page_request_invalid())?;
    if &decoded.tenant_scope_id != expected_tenant_scope_id {
        return Err(page_request_invalid());
    }
    Ok(decoded.after_run_id)
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

/// Strict, owner-validated admission request.
///
/// This type has no Serde implementation. Transports must pass their JSON bytes through
/// [`Self::decode_json`], which rejects duplicate keys, floats, wrong literals, unknown fields,
/// and invalid checked identities before authorization.
#[derive(Clone)]
pub struct AdmitRunRequest {
    validated: PlainCanonicalJsonBytes,
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
    /// Decodes ordinary transport JSON and mints one owner-validated request.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, PublicError> {
        let canonical = canonicalize_transport_json(
            bytes,
            "AdmissionRequestInvalid",
            "Admission request JSON is invalid",
        )?;
        let validated = canonical.clone();
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
        mfm_spec::CanonicalJsonValue::from_canonical_json(input_bytes.as_bytes()).map_err(
            |_| {
                invalid_request(
                    "AdmissionRequestInvalid",
                    "Admission request input is not canonical JSON",
                )
            },
        )?;
        let input_value = serde_json::from_slice::<Value>(input_bytes.as_bytes())
            .ok()
            .and_then(|value| canonical_value_from_json(&value))
            .ok_or_else(|| {
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
        let validated = PlainCanonicalJsonBytes::from_canonical_json_slice(
            mfm_canonical::CanonicalJsonBytes::from_value(&value).as_bytes(),
        )
        .map_err(|_| {
            invalid_request(
                "AdmissionRequestInvalid",
                "Admission request does not match the frozen contract",
            )
        })?;
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

    /// Returns the exact canonical request bytes this owner admitted.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AdmissionStatusWire {
    NewlyAdmitted,
    Attached,
    OutcomeUnknown,
}

impl From<AdmissionStatus> for AdmissionStatusWire {
    fn from(value: AdmissionStatus) -> Self {
        match value {
            AdmissionStatus::NewlyAdmitted => Self::NewlyAdmitted,
            AdmissionStatus::Attached => Self::Attached,
            AdmissionStatus::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}

impl From<AdmissionStatusWire> for AdmissionStatus {
    fn from(value: AdmissionStatusWire) -> Self {
        match value {
            AdmissionStatusWire::NewlyAdmitted => Self::NewlyAdmitted,
            AdmissionStatusWire::Attached => Self::Attached,
            AdmissionStatusWire::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmitRunResponseWire {
    version: String,
    run_id: RunId,
    admission: AdmissionStatusWire,
    entry_point_id: EntryPointId,
    entry_point_operation_id: StableId,
    invocation_identity: InvocationIdentity,
    planning_profile_ref: ContentRef,
}

impl AdmitRunResponseWire {
    fn validate(&self) -> Result<(), PublicError> {
        let planning_profile_schema =
            PlanningProfile::schema_id().map_err(|_| canonical_response_invalid())?;
        if self.version != ADMIT_RUN_RESPONSE_CONTRACT
            || self.planning_profile_ref.schema_id() != &planning_profile_schema
        {
            return Err(canonical_response_invalid());
        }
        Ok(())
    }
}

/// Exact owner-validated admission response.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmitRunResponse {
    canonical: PlainCanonicalJsonBytes,
    admission: AdmissionStatus,
}

impl AdmitRunResponse {
    /// Strictly decodes the one current canonical admission-response language.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self, PublicError> {
        let (canonical, wire) = decode_exact_response::<AdmitRunResponseWire>(bytes)?;
        wire.validate()?;
        Ok(Self {
            canonical,
            admission: wire.admission.into(),
        })
    }

    pub(crate) fn new(
        run_id: &RunId,
        admission: AdmissionStatus,
        entry_point_id: &EntryPointId,
        entry_point_operation_id: &StableId,
        invocation_identity: &InvocationIdentity,
        planning_profile_ref: &ContentRef,
    ) -> Result<Self, PublicError> {
        let wire = AdmitRunResponseWire {
            version: ADMIT_RUN_RESPONSE_CONTRACT.to_owned(),
            run_id: run_id.clone(),
            admission: admission.into(),
            entry_point_id: entry_point_id.clone(),
            entry_point_operation_id: entry_point_operation_id.clone(),
            invocation_identity: invocation_identity.clone(),
            planning_profile_ref: planning_profile_ref.clone(),
        };
        wire.validate()?;
        let canonical = encode_exact_response(&wire)?;
        Self::strict_decode(canonical.as_bytes())
    }

    /// Returns exact canonical response bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }

    /// Returns the admission observation used by transport status mapping.
    pub const fn admission(&self) -> AdmissionStatus {
        self.admission
    }
}

impl std::fmt::Debug for AdmitRunResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmitRunResponse")
            .field("admission", &self.admission)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DriveWaitingReasonWire {
    OperationalBlock,
    IntegrityBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum DriveResponseWire {
    Advanced {
        version: String,
        run_id: RunId,
        journal_head: JournalHead,
        reason: (),
    },
    Waiting {
        version: String,
        run_id: RunId,
        journal_head: JournalHead,
        reason: DriveWaitingReasonWire,
    },
    Closed {
        version: String,
        run_id: RunId,
        journal_head: JournalHead,
        reason: (),
    },
}

impl DriveResponseWire {
    fn validate(&self) -> Result<(), PublicError> {
        let (version, journal_head) = match self {
            Self::Advanced {
                version,
                journal_head,
                ..
            }
            | Self::Waiting {
                version,
                journal_head,
                ..
            }
            | Self::Closed {
                version,
                journal_head,
                ..
            } => (version, journal_head),
        };
        if version != DRIVE_RESPONSE_CONTRACT || journal_head.run_sequence == 0 {
            return Err(canonical_response_invalid());
        }
        Ok(())
    }
}

/// Exact owner-validated result of one run action.
#[derive(Clone, PartialEq, Eq)]
pub struct DriveResponse {
    canonical: PlainCanonicalJsonBytes,
}

impl DriveResponse {
    /// Strictly decodes the one current canonical drive-response language.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self, PublicError> {
        let (canonical, wire) = decode_exact_response::<DriveResponseWire>(bytes)?;
        wire.validate()?;
        Ok(Self { canonical })
    }

    pub(crate) fn from_runtime(
        run_id: &RunId,
        outcome: mfm_runtime::structured::DriveOutcome,
        journal_head: &JournalHead,
    ) -> Result<Self, PublicError> {
        let version = DRIVE_RESPONSE_CONTRACT.to_owned();
        let wire = match outcome {
            mfm_runtime::structured::DriveOutcome::TransitionCommitted { closed: true }
            | mfm_runtime::structured::DriveOutcome::Closed => DriveResponseWire::Closed {
                version,
                run_id: run_id.clone(),
                journal_head: journal_head.clone(),
                reason: (),
            },
            mfm_runtime::structured::DriveOutcome::TransitionCommitted { closed: false }
            | mfm_runtime::structured::DriveOutcome::AccessObserved
            | mfm_runtime::structured::DriveOutcome::ConcurrentProgress => {
                DriveResponseWire::Advanced {
                    version,
                    run_id: run_id.clone(),
                    journal_head: journal_head.clone(),
                    reason: (),
                }
            }
            mfm_runtime::structured::DriveOutcome::PossibleEntry(_) => DriveResponseWire::Waiting {
                version,
                run_id: run_id.clone(),
                journal_head: journal_head.clone(),
                reason: DriveWaitingReasonWire::OperationalBlock,
            },
            mfm_runtime::structured::DriveOutcome::BlockedIntegrity => DriveResponseWire::Waiting {
                version,
                run_id: run_id.clone(),
                journal_head: journal_head.clone(),
                reason: DriveWaitingReasonWire::IntegrityBlock,
            },
        };
        wire.validate()?;
        let canonical = encode_exact_response(&wire)?;
        Self::strict_decode(canonical.as_bytes())
    }

    /// Returns exact canonical response bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
}

impl std::fmt::Debug for DriveResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DriveResponse")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PublicRunStatusWire {
    Actionable,
    PossibleEntry,
    BlockedIntegrity,
    Closed,
}

impl PublicRunStatusWire {
    fn parse(value: &str) -> Result<Self, PublicError> {
        match value {
            "actionable" => Ok(Self::Actionable),
            "possible_entry" => Ok(Self::PossibleEntry),
            "blocked_integrity" => Ok(Self::BlockedIntegrity),
            "closed" => Ok(Self::Closed),
            _ => Err(canonical_response_invalid()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PublicOutcomeKindWire {
    Success,
    Failure,
}

impl PublicOutcomeKindWire {
    fn parse(value: &str) -> Result<Self, PublicError> {
        match value {
            "success" => Ok(Self::Success),
            "failure" => Ok(Self::Failure),
            _ => Err(canonical_response_invalid()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicOperationOutcomeWire {
    kind: PublicOutcomeKindWire,
    value_ref: LexicalValueRef,
    value: mfm_spec::CanonicalJsonValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicRunViewWire {
    version: String,
    run_id: RunId,
    tenant_scope_id: TenantScopeId,
    invocation_identity: InvocationIdentity,
    entry_point_operation_id: StableId,
    journal_head: JournalHead,
    semantic_head: SemanticHead,
    status: PublicRunStatusWire,
    outcome: Option<PublicOperationOutcomeWire>,
}

impl PublicRunViewWire {
    fn validate(&self) -> Result<(), PublicError> {
        let semantic_ref = semantic_record_ref(&self.semantic_head);
        if self.version != PUBLIC_RUN_VIEW_CONTRACT
            || self.journal_head.run_sequence == 0
            || semantic_ref.run_sequence == 0
            || semantic_ref.run_id != self.run_id
            || semantic_ref.run_sequence > self.journal_head.run_sequence
            || matches!(self.status, PublicRunStatusWire::Closed) != self.outcome.is_some()
        {
            return Err(canonical_response_invalid());
        }
        if let Some(outcome) = &self.outcome {
            let canonical = mfm_journal::structured::canonical_json(&outcome.value)
                .map_err(|_| canonical_response_invalid())?;
            let digest = ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(canonical.as_bytes()),
            );
            if outcome.value_ref.value.value_ref.content_digest() != &digest {
                return Err(canonical_response_invalid());
            }
        }
        Ok(())
    }
}

fn semantic_record_ref(head: &SemanticHead) -> &RecordRef {
    match head {
        SemanticHead::Genesis { admission_ref, .. } => admission_ref,
        SemanticHead::Transition { transition_ref, .. } => transition_ref,
    }
}

/// Exact owner-validated ordinary public run view.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicRunView {
    canonical: PlainCanonicalJsonBytes,
}

impl PublicRunView {
    /// Strictly decodes the one current canonical public-run-view language.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self, PublicError> {
        let (canonical, wire) = decode_exact_response::<PublicRunViewWire>(bytes)?;
        wire.validate()?;
        Ok(Self { canonical })
    }

    pub(crate) fn from_structured(
        evidence: &mfm_store::structured::PublicRunEvidence,
    ) -> Result<Self, PublicError> {
        let outcome = mfm_replay::structured::project_operation_outcome(evidence)
            .map_err(|_| PublicError::replay_verification_failed())?
            .map(|outcome| {
                let value = serde_json::from_slice(outcome.canonical_value().as_bytes())
                    .map_err(|_| PublicError::replay_verification_failed())?;
                Ok::<_, PublicError>(PublicOperationOutcomeWire {
                    kind: PublicOutcomeKindWire::parse(outcome.kind())?,
                    value_ref: outcome.value().clone(),
                    value,
                })
            })
            .transpose()?;
        let wire = PublicRunViewWire {
            version: PUBLIC_RUN_VIEW_CONTRACT.to_owned(),
            run_id: evidence.run_id().clone(),
            tenant_scope_id: evidence.header().tenant_scope_id().clone(),
            invocation_identity: evidence.header().invocation_identity().clone(),
            entry_point_operation_id: evidence.header().entry_point_operation_id().clone(),
            journal_head: evidence.journal_head().clone(),
            semantic_head: evidence.semantic_head().clone(),
            status: PublicRunStatusWire::parse(evidence.status().as_str())?,
            outcome,
        };
        wire.validate()?;
        let canonical = encode_exact_response(&wire)?;
        Self::strict_decode(canonical.as_bytes())
    }

    /// Returns exact canonical response bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
}

impl std::fmt::Debug for PublicRunView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PublicRunView")
            .finish_non_exhaustive()
    }
}

/// Public recorded-history replay mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    /// Verify recorded history without callbacks.
    Verify,
}

impl ReplayMode {
    /// Parses the exact transport spelling this owner admits.
    pub fn parse(value: &str) -> Result<Self, PublicError> {
        let mode = match value {
            "verify" => Self::Verify,
            _ => {
                return Err(invalid_request(
                    "ReplayModeInvalid",
                    "Replay mode must be verify",
                ))
            }
        };

        Ok(mode)
    }

    /// Returns the exact transport spelling.
    pub const fn as_transport_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
        }
    }
}

/// Affine asynchronous reader used for portable export streams.
pub type ExportAsyncReader = Pin<Box<dyn AsyncRead + Send + Unpin + 'static>>;

/// Checked replay request.
#[derive(Debug)]
pub enum ReplayRequest {
    /// Verify recorded history without any supplied export.
    Verify,
}

impl ReplayRequest {
    /// Returns the requested replay mode.
    pub const fn mode(&self) -> ReplayMode {
        match self {
            Self::Verify => ReplayMode::Verify,
        }
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

    /// Returns the owner-derived stream schema identity.
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
    use static_assertions::assert_not_impl_any;

    use super::{
        decode_access_audit_page_request, decode_transition_trace_page_request,
        encode_inspection_cursor, AdmissionStatus, AdmitRunRequest, AdmitRunResponse,
        DriveResponse, EffectEntryAttentionEntry, EffectEntryAttentionPage, ExportedRun,
        InspectionPurpose, PageRequest, PublicRunView, ReplayMode, ReplayRequest, RunId,
        DEFAULT_PAGE_LIMIT, INSPECTION_CURSOR_PREFIX, MAX_CURSOR_ENCODED_BYTES, MAX_PAGE_LIMIT,
    };

    assert_not_impl_any!(ExportedRun: Clone, Copy);
    assert_not_impl_any!(ReplayRequest: Clone, Copy);
    assert_not_impl_any!(
        EffectEntryAttentionEntry:
            serde::Serialize,
            serde::Deserialize<'static>,
            serde::de::DeserializeOwned
    );
    assert_not_impl_any!(
        EffectEntryAttentionPage:
            serde::Serialize,
            serde::Deserialize<'static>,
            serde::de::DeserializeOwned
    );

    /// The minimum admission request wire form.
    const ADMIT_RUN_REQUEST_WIRE: &str = r#"{"entry_point_id":"mfm.portfolio/snapshot@1","input":{},"invocation_identity":"00000000-0000-4000-8000-000000000000","version":"mfm.admit-run-request.v1"}"#;

    /// One advanced drive response wire form.
    const DRIVE_RESPONSE_ADVANCED_WIRE: &str = r#"{"journal_head":{"commit_digest":"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","run_sequence":1},"kind":"advanced","reason":null,"run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","version":"mfm.drive-response.v1"}"#;

    /// One waiting drive response wire form.
    const DRIVE_RESPONSE_WAITING_WIRE: &str = r#"{"journal_head":{"commit_digest":"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","run_sequence":1},"kind":"waiting","reason":"operational_block","run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","version":"mfm.drive-response.v1"}"#;

    /// One closed drive response wire form.
    const DRIVE_RESPONSE_CLOSED_WIRE: &str = r#"{"journal_head":{"commit_digest":"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","run_sequence":1},"kind":"closed","reason":null,"run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","version":"mfm.drive-response.v1"}"#;

    /// The minimum public run view wire form.
    const PUBLIC_RUN_VIEW_WIRE: &str = r#"{"entry_point_operation_id":"mfm.portfolio/snapshot","invocation_identity":"00000000-0000-4000-8000-000000000000","journal_head":{"commit_digest":"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","run_sequence":1},"outcome":null,"run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","semantic_head":{"admission_ref":{"ordinal":0,"record_hash":"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","run_sequence":1},"kind":"genesis","semantic_state_digest":"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"},"status":"actionable","tenant_scope_id":"mfm.tenant_scope.v1:11111111111111111111111111111111","version":"mfm.public-run-view.v1"}"#;

    fn admit_run_response_wire() -> Vec<u8> {
        let planning_profile_schema =
            <mfm_spec::PlanningProfile as mfm_values::PersistedSchema>::schema_id()
                .expect("planning profile schema");
        canonical_test_bytes(&serde_json::json!({
            "admission": "attached",
            "entry_point_id": "mfm.portfolio/snapshot@1",
            "entry_point_operation_id": "mfm.portfolio/snapshot",
            "invocation_identity": "00000000-0000-4000-8000-000000000000",
            "planning_profile_ref": {
                "content_digest": format!("content:sha256-v1:{}", "1".repeat(64)),
                "schema_id": planning_profile_schema,
            },
            "run_id": format!("run:sha256-jcs-v1:{}", "0".repeat(64)),
            "version": "mfm.admit-run-response.v1",
        }))
    }

    fn canonical_test_bytes(value: &serde_json::Value) -> Vec<u8> {
        mfm_journal::structured::canonical_json(value)
            .expect("canonical test response")
            .as_bytes()
            .to_vec()
    }

    /// Every app-owned wire value decodes to its exact canonical bytes.
    #[test]
    fn app_owned_wire_values_round_trip_their_exact_canonical_bytes() {
        let request = ADMIT_RUN_REQUEST_WIRE.as_bytes();
        assert_eq!(
            AdmitRunRequest::decode_json(request)
                .expect("admission request")
                .as_bytes(),
            request
        );

        let admission_wire = admit_run_response_wire();
        let admission =
            AdmitRunResponse::strict_decode(&admission_wire).expect("admission response");
        assert_eq!(admission.admission(), AdmissionStatus::Attached);
        assert_eq!(admission.as_bytes(), admission_wire);

        for wire in [
            DRIVE_RESPONSE_ADVANCED_WIRE,
            DRIVE_RESPONSE_WAITING_WIRE,
            DRIVE_RESPONSE_CLOSED_WIRE,
        ] {
            assert_eq!(
                DriveResponse::strict_decode(wire.as_bytes())
                    .expect("drive response")
                    .as_bytes(),
                wire.as_bytes()
            );
        }

        assert_eq!(
            PublicRunView::strict_decode(PUBLIC_RUN_VIEW_WIRE.as_bytes())
                .expect("public run view")
                .as_bytes(),
            PUBLIC_RUN_VIEW_WIRE.as_bytes()
        );
    }

    #[test]
    fn application_replay_mode_accepts_only_the_current_spelling() {
        assert_eq!(
            ReplayMode::parse("verify").expect("current mode"),
            ReplayMode::Verify
        );
        assert!(ReplayMode::parse("reproduce").is_err());
        assert!(ReplayMode::parse("compare_current").is_err());
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
        let value: serde_json::Value =
            serde_json::from_str(DRIVE_RESPONSE_ADVANCED_WIRE).expect("drive response JSON");
        let mut object = value.as_object().expect("drive response object").clone();
        let kind = object.remove("kind").expect("drive response kind");
        object.insert("outcome".to_owned(), kind);
        let superseded = serde_json::to_vec(&object).expect("superseded drive response JSON");

        assert!(DriveResponse::strict_decode(&superseded).is_err());
    }

    #[test]
    fn response_owners_reject_incomplete_unknown_and_cross_owner_bytes() {
        let garbage = br#"{"kind":"garbage"}"#;
        assert!(AdmitRunResponse::strict_decode(garbage).is_err());
        assert!(DriveResponse::strict_decode(garbage).is_err());
        assert!(PublicRunView::strict_decode(garbage).is_err());

        assert!(AdmitRunResponse::strict_decode(DRIVE_RESPONSE_ADVANCED_WIRE.as_bytes()).is_err());
        assert!(PublicRunView::strict_decode(DRIVE_RESPONSE_ADVANCED_WIRE.as_bytes()).is_err());

        let mut drive: serde_json::Value =
            serde_json::from_str(DRIVE_RESPONSE_ADVANCED_WIRE).expect("drive response");
        drive["reason"] = serde_json::json!("operational_block");
        assert!(DriveResponse::strict_decode(&canonical_test_bytes(&drive)).is_err());
        drive["reason"] = serde_json::Value::Null;
        drive["extra"] = serde_json::json!(true);
        assert!(DriveResponse::strict_decode(&canonical_test_bytes(&drive)).is_err());

        let mut admission: serde_json::Value =
            serde_json::from_slice(&admit_run_response_wire()).expect("admission response");
        admission["planning_profile_ref"]["schema_id"] = serde_json::json!(format!(
            "schema:mfm.foreign:1:sha256-jcs-v1:{}",
            "2".repeat(64)
        ));
        assert!(AdmitRunResponse::strict_decode(&canonical_test_bytes(&admission)).is_err());

        let mut public: serde_json::Value =
            serde_json::from_str(PUBLIC_RUN_VIEW_WIRE).expect("public run view");
        public["status"] = serde_json::json!("closed");
        assert!(PublicRunView::strict_decode(&canonical_test_bytes(&public)).is_err());
        public["status"] = serde_json::json!("actionable");
        public
            .as_object_mut()
            .expect("public run view object")
            .remove("version");
        assert!(PublicRunView::strict_decode(&canonical_test_bytes(&public)).is_err());
    }
}
