//! Append-only configured-value history outside per-run history families.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId, StoreScopeId,
    TenantScopeId,
};
use mfm_journal::structured::HistoryObject;
use mfm_values::PersistedObjectPayload;
use serde::{Deserialize, Serialize};

use super::validated_append::ValidatedConfigurationAppend;
use super::{ProposedCanonicalValue, StructuredStoreError};

/// Maximum bytes in one canonical structured configuration revision.
pub const MAX_CONFIGURATION_REVISION_BYTES: usize = 16777216;

/// Immutable routing key for one tenant-scoped configured-value stream.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationStreamKey {
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
    entry_point_operation_id: StableId,
    target_id: StableId,
}

impl ConfigurationStreamKey {
    /// Constructs one exact configured-value stream key.
    pub const fn new(
        store_scope_id: StoreScopeId,
        tenant_scope_id: TenantScopeId,
        entry_point_operation_id: StableId,
        target_id: StableId,
    ) -> Self {
        Self {
            store_scope_id,
            tenant_scope_id,
            entry_point_operation_id,
            target_id,
        }
    }

    /// Returns the qualified store lineage.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Returns the app-authorized tenant.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the published operation identity.
    pub const fn entry_point_operation_id(&self) -> &StableId {
        &self.entry_point_operation_id
    }

    /// Returns the configured target identity.
    pub const fn target_id(&self) -> &StableId {
        &self.target_id
    }
}

/// One immutable configured-value revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationRevision {
    key: ConfigurationStreamKey,
    sequence: u64,
    predecessor_ref: Option<ContentRef>,
    append_request_id: AppendRequestId,
    value_contract_ref: ContentRef,
    value_ref: ContentRef,
    canonical_value: String,
}

impl mfm_values::PersistedSchema for ConfigurationRevision {
    fn schema_identity() -> mfm_values::Result<mfm_values::SchemaIdentity> {
        use mfm_values::{
            FieldDescriptor, SchemaIdentity, SchemaKind, SchemaShape, StringGrammar, ValueError,
        };

        let reference = SchemaShape::content_ref()?;
        let stream_key = SchemaShape::named_struct(vec![
            FieldDescriptor::required(
                "entry_point_operation_id",
                SchemaShape::identity_string(StringGrammar::StableId, 256),
            ),
            FieldDescriptor::required(
                "store_scope_id",
                SchemaShape::identity_string(StringGrammar::StoreScopeId, 64),
            ),
            FieldDescriptor::required(
                "target_id",
                SchemaShape::identity_string(StringGrammar::StableId, 256),
            ),
            FieldDescriptor::required(
                "tenant_scope_id",
                SchemaShape::identity_string(StringGrammar::TenantScopeId, 64),
            ),
        ])?;
        SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm.structured-configuration-revision",
            mfm_ids::SchemaVersion::new("1")
                .map_err(|error| ValueError::Identity(error.to_string()))?,
            SchemaShape::named_struct(vec![
                FieldDescriptor::required(
                    "append_request_id",
                    SchemaShape::identity_string(StringGrammar::StableId, 256),
                ),
                FieldDescriptor::required(
                    "canonical_value",
                    SchemaShape::BoundedString {
                        minimum_bytes: 1,
                        maximum_bytes: MAX_CONFIGURATION_REVISION_BYTES as u32,
                        grammar: StringGrammar::UnicodeScalarText,
                    },
                ),
                FieldDescriptor::required("key", stream_key),
                FieldDescriptor::required(
                    "predecessor_ref",
                    SchemaShape::Option(Box::new(reference.clone())),
                ),
                FieldDescriptor::required(
                    "sequence",
                    SchemaShape::UnsignedRange {
                        minimum: 1,
                        maximum: u64::MAX,
                    },
                ),
                FieldDescriptor::required("value_contract_ref", reference.clone()),
                FieldDescriptor::required("value_ref", reference),
            ])?,
        )
    }

    fn validate(&self) -> mfm_values::Result<()> {
        self.validate_payload()
            .map_err(|_| mfm_values::ValueError::SchemaShapeMismatch)
    }
}

impl PersistedObjectPayload for ConfigurationRevision {
    fn object_type() -> mfm_values::Result<StableId> {
        StableId::new(mfm_journal::structured::ADMISSION_CONFIGURATION_OBJECT_TYPE)
            .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

impl ConfigurationRevision {
    /// Returns the one owner-derived configuration-revision schema identity.
    ///
    /// Admission material and the configuration store share this identity, so
    /// the same revision bytes cannot carry two schema derivations.
    pub fn schema_id() -> Result<SchemaId, StructuredStoreError> {
        <Self as mfm_values::PersistedSchema>::schema_id().map_err(|_| invalid())
    }

    fn new(
        key: ConfigurationStreamKey,
        sequence: u64,
        predecessor_ref: Option<ContentRef>,
        append_request_id: AppendRequestId,
        value_contract_ref: ContentRef,
        value: &ProposedCanonicalValue,
    ) -> Result<Self, StructuredStoreError> {
        let canonical_value = value.canonical().as_str().to_owned();
        let value_ref = content_ref(configured_value_schema_id()?, canonical_value.as_bytes())?;
        let revision = Self {
            key,
            sequence,
            predecessor_ref,
            append_request_id,
            value_contract_ref,
            value_ref,
            canonical_value,
        };
        revision.validate_payload()?;
        Ok(revision)
    }

    fn validate_payload(&self) -> Result<(), StructuredStoreError> {
        if self.sequence == 0 {
            return Err(invalid());
        }
        if self.canonical_value.len() > MAX_CONFIGURATION_REVISION_BYTES {
            return Err(invalid());
        }
        let canonical =
            PlainCanonicalJsonBytes::from_canonical_json_slice(self.canonical_value.as_bytes())
                .map_err(|_| invalid())?;
        if content_ref(configured_value_schema_id()?, canonical.as_bytes())? != self.value_ref {
            return Err(invalid());
        }
        Ok(())
    }

    pub(crate) fn validate_for_ingress(&self) -> Result<(), StructuredStoreError> {
        self.validate_payload()
    }

    /// Returns the stream key.
    pub const fn key(&self) -> &ConfigurationStreamKey {
        &self.key
    }

    /// Returns the one-based revision sequence.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the exact predecessor revision, when any.
    pub const fn predecessor_ref(&self) -> Option<&ContentRef> {
        self.predecessor_ref.as_ref()
    }

    /// Returns the idempotent append request identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the retained value contract.
    pub const fn value_contract_ref(&self) -> &ContentRef {
        &self.value_contract_ref
    }

    /// Returns the content-addressed value.
    pub const fn value_ref(&self) -> &ContentRef {
        &self.value_ref
    }

    /// Returns exact canonical configured bytes.
    pub fn canonical_value(&self) -> &str {
        &self.canonical_value
    }
}

/// One owner-qualified configuration revision and its exact retained object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationRevisionObject {
    object: HistoryObject,
    decoded: ConfigurationRevision,
}

impl ConfigurationRevisionObject {
    fn from_revision(decoded: ConfigurationRevision) -> Result<Self, StructuredStoreError> {
        let object = HistoryObject::from_persisted(&decoded).map_err(|_| invalid())?;
        Ok(Self { object, decoded })
    }

    /// Qualifies one exact retained configuration object.
    #[doc(hidden)]
    pub fn from_object(object: HistoryObject) -> Result<Self, StructuredStoreError> {
        let decoded = object
            .decode_persisted::<ConfigurationRevision>()
            .map_err(|_| invalid())?;
        decoded.validate_payload()?;
        Ok(Self { object, decoded })
    }

    /// Returns the immutable decoded payload.
    pub const fn revision(&self) -> &ConfigurationRevision {
        &self.decoded
    }

    /// Returns the exact retained object.
    pub const fn object(&self) -> &HistoryObject {
        &self.object
    }

    /// Returns the sole content identity of this revision.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.object.content_ref
    }
}

/// Store-verified current configured value selected for admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedConfiguredValue {
    revision: ConfigurationRevisionObject,
}

impl VerifiedConfiguredValue {
    /// Returns the immutable selected revision.
    pub const fn revision(&self) -> &ConfigurationRevision {
        self.revision.revision()
    }

    /// Returns the exact qualified revision object used by admission.
    pub const fn revision_object(&self) -> &ConfigurationRevisionObject {
        &self.revision
    }

    /// Returns exact canonical value bytes.
    pub fn canonical_value(&self) -> &str {
        self.revision.revision().canonical_value()
    }
}

/// Complete raw configured-value prefix loaded by a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawConfigurationHistory {
    /// Selected stream.
    pub key: ConfigurationStreamKey,
    /// Independently persisted authoritative stream head.
    pub head: Option<ConfigurationHistoryHead>,
    /// Revisions in ascending sequence order.
    pub revisions: Vec<ConfigurationRevisionObject>,
}

/// Independently persisted authoritative head of one configured-value stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationHistoryHead {
    sequence: u64,
    object_ref: ContentRef,
}

impl ConfigurationHistoryHead {
    /// Constructs one physical head pointer loaded by a qualified backend.
    #[doc(hidden)]
    pub const fn new(sequence: u64, object_ref: ContentRef) -> Self {
        Self {
            sequence,
            object_ref,
        }
    }

    /// Returns the one-based sequence named by this head.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the exact revision named by this head.
    pub const fn object_ref(&self) -> &ContentRef {
        &self.object_ref
    }
}

/// Result of one exact-predecessor configured-value transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationBackendAppendOutcome {
    /// The exact revision was newly committed.
    NewlyCommitted(ConfigurationRevisionObject),
    /// The append identity already names the same revision.
    ExistingSame(ConfigurationRevisionObject),
    /// The durable predecessor differs.
    StaleHead,
    /// Commit acknowledgement must be resolved before retry.
    AcknowledgementUnknown,
}

/// Boxed configured-history backend operation.
pub type ConfigurationBackendFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, StructuredStoreError>> + Send + 'a>>;

/// Raw persistence seam for append-only configured-value streams.
pub trait ConfigurationHistoryBackend: Send + Sync + 'static {
    /// Returns the qualified store lineage.
    fn store_scope_id(&self) -> &StoreScopeId;

    /// Loads one exact immutable prefix.
    fn load<'a>(
        &'a self,
        key: &'a ConfigurationStreamKey,
    ) -> ConfigurationBackendFuture<'a, Option<RawConfigurationHistory>>;

    /// Captures the union of immutable revision and current-head streams in one snapshot.
    fn load_store_snapshot(&self) -> ConfigurationBackendFuture<'_, Vec<RawConfigurationHistory>>;

    /// Freshly revalidates the target authority after semantic qualification.
    fn validate_authority(&self) -> ConfigurationBackendFuture<'_, ()>;

    /// Atomically appends at the exact predecessor.
    fn append<'a>(
        &'a self,
        revision: ValidatedConfigurationAppend,
    ) -> ConfigurationBackendFuture<'a, ConfigurationBackendAppendOutcome>;
}

/// One append request held only by deployment configuration maintenance.
pub struct ConfigurationAppendRequest {
    key: ConfigurationStreamKey,
    expected_predecessor_ref: Option<ContentRef>,
    append_request_id: AppendRequestId,
    value_contract_ref: ContentRef,
    value: ProposedCanonicalValue,
}

impl ConfigurationAppendRequest {
    /// Constructs one exact-predecessor append request.
    pub const fn new(
        key: ConfigurationStreamKey,
        expected_predecessor_ref: Option<ContentRef>,
        append_request_id: AppendRequestId,
        value_contract_ref: ContentRef,
        value: ProposedCanonicalValue,
    ) -> Self {
        Self {
            key,
            expected_predecessor_ref,
            append_request_id,
            value_contract_ref,
            value,
        }
    }
}

/// Bounded retries for a SQL commit whose acknowledgement was lost. The
/// database either retained the identical append or it did not; retrying the
/// exact canonical revision resolves which, and the bound keeps an
/// unreachable backend from looping forever.
const MAX_CONFIGURATION_APPEND_RETRIES: usize = 8;

/// Non-cloneable deployment authority for configured-value appends.
pub struct ConfigurationHistoryWriter<B: ConfigurationHistoryBackend> {
    backend: Arc<B>,
}

impl<B: ConfigurationHistoryBackend> ConfigurationHistoryWriter<B> {
    async fn load_for_append(
        &self,
        key: &ConfigurationStreamKey,
    ) -> Result<Option<RawConfigurationHistory>, StructuredStoreError> {
        self.backend.load(key).await
    }

    async fn retry_ambiguous_append(
        &self,
        revision: &ConfigurationRevisionObject,
    ) -> Result<ConfigurationRevisionObject, StructuredStoreError> {
        for _ in 0..MAX_CONFIGURATION_APPEND_RETRIES {
            let outcome = self
                .backend
                .append(ValidatedConfigurationAppend::from_object(revision.clone())?)
                .await?;
            match outcome {
                ConfigurationBackendAppendOutcome::NewlyCommitted(returned)
                | ConfigurationBackendAppendOutcome::ExistingSame(returned)
                    if returned == *revision =>
                {
                    return Ok(revision.clone())
                }
                ConfigurationBackendAppendOutcome::NewlyCommitted(_)
                | ConfigurationBackendAppendOutcome::ExistingSame(_) => {
                    return Err(invalid());
                }
                ConfigurationBackendAppendOutcome::AcknowledgementUnknown => {}
                ConfigurationBackendAppendOutcome::StaleHead => {
                    let resolved = self.load_for_append(revision.revision().key()).await?;
                    match resolved.as_ref().and_then(|history| {
                        history_append_identity(
                            history.revisions.iter(),
                            revision.revision().append_request_id(),
                        )
                    }) {
                        Some(existing) if existing == revision => return Ok(revision.clone()),
                        Some(_) => return Err(StructuredStoreError::AppendConflict),
                        None if resolved.as_ref().is_none_or(|history| {
                            history
                                .head
                                .as_ref()
                                .map(ConfigurationHistoryHead::object_ref)
                                == revision.revision().predecessor_ref()
                        }) => {}
                        None => return Err(StructuredStoreError::StaleHead),
                    }
                }
            }
        }
        Err(StructuredStoreError::AcknowledgementUnknown)
    }

    /// Appends one exact revision or returns the existing byte-identical result.
    pub async fn append(
        &self,
        request: ConfigurationAppendRequest,
    ) -> Result<ConfigurationRevisionObject, StructuredStoreError> {
        require_store(self.backend.as_ref(), &request.key)?;
        let history = self.load_for_append(&request.key).await?;
        let verified = history
            .clone()
            .map(verify_configuration_history)
            .transpose()?;
        let current = verified
            .as_ref()
            .map(VerifiedConfiguredValue::revision_object);
        if let Some(existing) = history.as_ref().and_then(|history| {
            history_append_identity(history.revisions.iter(), &request.append_request_id)
        }) {
            return if existing.revision().value_contract_ref == request.value_contract_ref
                && existing.revision().canonical_value == request.value.canonical().as_str()
            {
                Ok(existing.clone())
            } else {
                Err(StructuredStoreError::AppendConflict)
            };
        }
        if current.map(ConfigurationRevisionObject::content_ref)
            != request.expected_predecessor_ref.as_ref()
        {
            return Err(StructuredStoreError::StaleHead);
        }
        let sequence = current.map_or(1, |revision| revision.revision().sequence + 1);
        let revision = ConfigurationRevisionObject::from_revision(ConfigurationRevision::new(
            request.key,
            sequence,
            request.expected_predecessor_ref,
            request.append_request_id,
            request.value_contract_ref,
            &request.value,
        )?)?;
        match self
            .backend
            .append(ValidatedConfigurationAppend::from_object(revision.clone())?)
            .await?
        {
            ConfigurationBackendAppendOutcome::NewlyCommitted(returned)
            | ConfigurationBackendAppendOutcome::ExistingSame(returned)
                if returned == revision =>
            {
                Ok(revision)
            }
            ConfigurationBackendAppendOutcome::NewlyCommitted(_)
            | ConfigurationBackendAppendOutcome::ExistingSame(_) => Err(invalid()),
            ConfigurationBackendAppendOutcome::StaleHead => {
                // A concurrent append with the same logical identity can win
                // after this writer's initial read but before the backend lock.
                // Reclassify the unchanged identity from a fresh read so a
                // race cannot turn an append conflict into an unrelated stale
                // predecessor result.
                for _ in 0..MAX_CONFIGURATION_APPEND_RETRIES {
                    let resolved = self.load_for_append(revision.revision().key()).await?;
                    match resolved.as_ref().and_then(|history| {
                        history_append_identity(
                            history.revisions.iter(),
                            revision.revision().append_request_id(),
                        )
                    }) {
                        Some(existing) if existing == &revision => return Ok(revision),
                        Some(_) => return Err(StructuredStoreError::AppendConflict),
                        None => match self
                            .backend
                            .append(ValidatedConfigurationAppend::from_object(revision.clone())?)
                            .await?
                        {
                            ConfigurationBackendAppendOutcome::NewlyCommitted(returned)
                            | ConfigurationBackendAppendOutcome::ExistingSame(returned)
                                if returned == revision =>
                            {
                                return Ok(revision)
                            }
                            ConfigurationBackendAppendOutcome::NewlyCommitted(_)
                            | ConfigurationBackendAppendOutcome::ExistingSame(_) => {
                                return Err(invalid());
                            }
                            ConfigurationBackendAppendOutcome::StaleHead => {}
                            ConfigurationBackendAppendOutcome::AcknowledgementUnknown => {}
                        },
                    }
                    // SQL commit and external acknowledgement are separate operations. Give the
                    // winner a bounded scheduling window before the next classification attempt.
                }
                Err(StructuredStoreError::StaleHead)
            }
            ConfigurationBackendAppendOutcome::AcknowledgementUnknown => {
                // Reconnect through the append authority, reacquire its lock,
                // and retry the identical canonical revision. The backend's
                // idempotent branch acknowledges a prepared durable successor;
                // a missing row retries the prepared predecessor. No external
                // operation is reinvoked with different bytes.
                self.retry_ambiguous_append(&revision).await
            }
        }
    }
}

/// Cloneable resolve-only configured-value authority used by application admission.
#[derive(Clone)]
pub struct ConfigurationHistoryReader<B: ConfigurationHistoryBackend> {
    backend: Arc<B>,
}

impl<B: ConfigurationHistoryBackend> ConfigurationHistoryReader<B> {
    /// Resolves and verifies the current exact-contract revision.
    pub async fn resolve(
        &self,
        key: &ConfigurationStreamKey,
        expected_value_contract_ref: &ContentRef,
    ) -> Result<VerifiedConfiguredValue, StructuredStoreError> {
        require_store(self.backend.as_ref(), key)?;
        let history = self
            .backend
            .load(key)
            .await?
            .ok_or(StructuredStoreError::RunNotFound)?;
        let verified = verify_configuration_history(history)?;
        if verified.revision().value_contract_ref != *expected_value_contract_ref {
            return Err(invalid());
        }
        Ok(verified)
    }
}

/// One-shot configured-history assembly separating append and resolve authority.
pub struct ConfigurationHistoryStore<B: ConfigurationHistoryBackend> {
    backend: Arc<B>,
}

impl<B: ConfigurationHistoryBackend> ConfigurationHistoryStore<B> {
    fn from_qualified(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }

    /// Separates deployment append authority from application resolve authority.
    pub fn into_authorities(
        self,
    ) -> (ConfigurationHistoryWriter<B>, ConfigurationHistoryReader<B>) {
        (
            ConfigurationHistoryWriter {
                backend: Arc::clone(&self.backend),
            },
            ConfigurationHistoryReader {
                backend: self.backend,
            },
        )
    }
}

/// Qualifies every configured-value stream before releasing either authority.
pub async fn qualify_and_open_configuration_history<B: ConfigurationHistoryBackend>(
    backend: B,
) -> Result<ConfigurationHistoryStore<B>, StructuredStoreError> {
    for history in backend.load_store_snapshot().await? {
        verify_configuration_history(history)?;
    }
    backend.validate_authority().await?;
    Ok(ConfigurationHistoryStore::from_qualified(backend))
}

/// In-memory conformance backend for configured-value history.
#[derive(Clone)]
pub struct MemoryConfigurationHistoryBackend {
    store_scope_id: StoreScopeId,
    state: Arc<Mutex<MemoryConfigurationHistoryState>>,
}

#[derive(Default)]
struct MemoryConfigurationHistoryState {
    revisions: BTreeMap<ConfigurationStreamKey, Vec<ConfigurationRevisionObject>>,
    heads: BTreeMap<ConfigurationStreamKey, ConfigurationHistoryHead>,
}

impl MemoryConfigurationHistoryBackend {
    /// Creates one empty qualified in-memory configured-history backend.
    pub fn new(store_scope_id: StoreScopeId) -> Self {
        Self {
            store_scope_id,
            state: Arc::new(Mutex::new(MemoryConfigurationHistoryState::default())),
        }
    }
}

impl mfm_authority_seal::ValidatedAppendConsumerSeal for MemoryConfigurationHistoryBackend {}

impl ConfigurationHistoryBackend for MemoryConfigurationHistoryBackend {
    fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    fn load<'a>(
        &'a self,
        key: &'a ConfigurationStreamKey,
    ) -> ConfigurationBackendFuture<'a, Option<RawConfigurationHistory>> {
        Box::pin(async move {
            let state = self
                .state
                .lock()
                .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let revisions = state.revisions.get(key).cloned().unwrap_or_default();
            let head = state.heads.get(key).cloned();
            if revisions.is_empty() && head.is_none() {
                Ok(None)
            } else {
                Ok(Some(RawConfigurationHistory {
                    key: key.clone(),
                    head,
                    revisions,
                }))
            }
        })
    }

    fn load_store_snapshot(&self) -> ConfigurationBackendFuture<'_, Vec<RawConfigurationHistory>> {
        Box::pin(async move {
            let state = self
                .state
                .lock()
                .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let keys = state
                .revisions
                .keys()
                .chain(state.heads.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            Ok(keys
                .into_iter()
                .map(|key| RawConfigurationHistory {
                    revisions: state.revisions.get(&key).cloned().unwrap_or_default(),
                    head: state.heads.get(&key).cloned(),
                    key,
                })
                .collect())
        })
    }

    fn validate_authority(&self) -> ConfigurationBackendFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn append<'a>(
        &'a self,
        command: ValidatedConfigurationAppend,
    ) -> ConfigurationBackendFuture<'a, ConfigurationBackendAppendOutcome> {
        Box::pin(async move {
            let (revision, expected_head, successor_head) = command.into_parts(self);
            let payload = revision.revision();
            let mut state = self
                .state
                .lock()
                .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let history = state
                .revisions
                .get(&payload.key)
                .cloned()
                .unwrap_or_default();
            let head = state.heads.get(&payload.key).cloned();
            if history.is_empty() != head.is_none() {
                return Err(invalid());
            }
            if let Some(existing) = history
                .iter()
                .find(|existing| existing.revision().append_request_id == payload.append_request_id)
            {
                return if existing == &revision {
                    Ok(ConfigurationBackendAppendOutcome::ExistingSame(
                        existing.clone(),
                    ))
                } else {
                    Err(StructuredStoreError::AppendConflict)
                };
            }
            if head != expected_head {
                return Ok(ConfigurationBackendAppendOutcome::StaleHead);
            }
            state
                .revisions
                .entry(payload.key.clone())
                .or_default()
                .push(revision.clone());
            state.heads.insert(payload.key.clone(), successor_head);
            Ok(ConfigurationBackendAppendOutcome::NewlyCommitted(revision))
        })
    }
}

/// Callback-free validation shared with qualified physical backends before mutation.
#[doc(hidden)]
pub(crate) fn verify_configuration_history(
    history: RawConfigurationHistory,
) -> Result<VerifiedConfiguredValue, StructuredStoreError> {
    let head = history.head.as_ref().ok_or_else(invalid)?;
    if history.revisions.is_empty() {
        return Err(invalid());
    }
    let mut predecessor = None;
    let mut append_ids = BTreeSet::new();
    let mut expected_contract = None;
    for (index, revision) in history.revisions.iter().enumerate() {
        revision.revision().validate_payload()?;
        let payload = revision.revision();
        if payload.key != history.key
            || payload.sequence != u64::try_from(index + 1).map_err(|_| invalid())?
            || payload.predecessor_ref != predecessor
            || !append_ids.insert(payload.append_request_id.clone())
            || expected_contract.get_or_insert_with(|| payload.value_contract_ref.clone())
                != &payload.value_contract_ref
        {
            return Err(invalid());
        }
        predecessor = Some(revision.content_ref().clone());
    }
    let revision = history.revisions.last().cloned().ok_or_else(invalid)?;
    if history.revisions.len() != usize::try_from(head.sequence).map_err(|_| invalid())?
        || revision.revision().sequence != head.sequence
        || revision.content_ref() != head.object_ref()
    {
        return Err(invalid());
    }
    Ok(VerifiedConfiguredValue { revision })
}

fn history_append_identity<'a>(
    revisions: impl Iterator<Item = &'a ConfigurationRevisionObject>,
    append_request_id: &AppendRequestId,
) -> Option<&'a ConfigurationRevisionObject> {
    revisions
        .into_iter()
        .find(|revision| revision.revision().append_request_id == *append_request_id)
}

fn require_store(
    backend: &impl ConfigurationHistoryBackend,
    key: &ConfigurationStreamKey,
) -> Result<(), StructuredStoreError> {
    if key.store_scope_id != *backend.store_scope_id() {
        return Err(invalid());
    }
    Ok(())
}

/// Returns the owner-derived identity of one retained configured value.
///
/// A configured value is caller-authored canonical JSON, so its retained shape
/// is the closed float-free terminal; the exact value contract is carried
/// separately by the revision.
fn configured_value_schema_id() -> Result<SchemaId, StructuredStoreError> {
    mfm_values::SchemaIdentity::new(
        mfm_values::SchemaKind::PersistedContract,
        None,
        "mfm.structured-configured-value",
        mfm_ids::SchemaVersion::new("1").map_err(|_| invalid())?,
        mfm_values::SchemaShape::CanonicalJsonTerminal {
            profile: mfm_values::CanonicalJsonProfile::GeneralFloatFree,
        },
    )
    .and_then(|identity| identity.schema_id())
    .map_err(|_| invalid())
}

fn content_ref(schema_id: SchemaId, bytes: &[u8]) -> Result<ContentRef, StructuredStoreError> {
    ContentRef::new(
        schema_id,
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(bytes)),
    )
    .map_err(|_| invalid())
}

const fn invalid() -> StructuredStoreError {
    StructuredStoreError::InvalidHistory
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn store_scope() -> StoreScopeId {
        StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "1".repeat(32)))
            .expect("store scope")
    }

    fn tenant(discriminator: char) -> TenantScopeId {
        TenantScopeId::new(format!(
            "{}{}",
            TenantScopeId::PREFIX,
            discriminator.to_string().repeat(32)
        ))
        .expect("tenant")
    }

    fn key(discriminator: char) -> ConfigurationStreamKey {
        ConfigurationStreamKey::new(
            store_scope(),
            tenant(discriminator),
            StableId::new("mfm.fixture/configured-operation").expect("operation"),
            StableId::new("mfm.fixture/configured-target").expect("target"),
        )
    }

    fn contract() -> ContentRef {
        content_ref(
            SchemaId::new(
                "mfm.fixture-configured-contract",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"mfm.fixture-configured-contract"),
            )
            .expect("fixture contract schema"),
            b"contract",
        )
        .expect("contract")
    }

    #[derive(Clone)]
    struct FlakyConfigurationBackend {
        inner: MemoryConfigurationHistoryBackend,
        transient_loads: Arc<AtomicUsize>,
        ambiguous_append: Arc<AtomicBool>,
    }

    impl ConfigurationHistoryBackend for FlakyConfigurationBackend {
        fn store_scope_id(&self) -> &StoreScopeId {
            self.inner.store_scope_id()
        }

        fn load<'a>(
            &'a self,
            key: &'a ConfigurationStreamKey,
        ) -> ConfigurationBackendFuture<'a, Option<RawConfigurationHistory>> {
            Box::pin(async move {
                if self
                    .transient_loads
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok()
                {
                    return Err(StructuredStoreError::InvalidHistory);
                }
                self.inner.load(key).await
            })
        }

        fn load_store_snapshot(
            &self,
        ) -> ConfigurationBackendFuture<'_, Vec<RawConfigurationHistory>> {
            self.inner.load_store_snapshot()
        }

        fn validate_authority(&self) -> ConfigurationBackendFuture<'_, ()> {
            self.inner.validate_authority()
        }

        fn append<'a>(
            &'a self,
            revision: ValidatedConfigurationAppend,
        ) -> ConfigurationBackendFuture<'a, ConfigurationBackendAppendOutcome> {
            Box::pin(async move {
                if self.ambiguous_append.swap(false, Ordering::AcqRel) {
                    // Persist the exact row, then hide the acknowledgement so
                    // the writer must recover through the backend's idempotent
                    // existing-same path rather than treating it as a fresh
                    // append.
                    self.inner.append(revision).await?;
                    return Ok(ConfigurationBackendAppendOutcome::AcknowledgementUnknown);
                }
                self.inner.append(revision).await
            })
        }
    }

    fn flaky_backend(
        inner: MemoryConfigurationHistoryBackend,
        transient_loads: usize,
        ambiguous_append: bool,
    ) -> FlakyConfigurationBackend {
        FlakyConfigurationBackend {
            inner,
            transient_loads: Arc::new(AtomicUsize::new(transient_loads)),
            ambiguous_append: Arc::new(AtomicBool::new(ambiguous_append)),
        }
    }

    #[test]
    fn configuration_revision_accepts_exact_budget_and_rejects_one_over() {
        let exact_json = format!(
            "\"{}\"",
            "x".repeat(MAX_CONFIGURATION_REVISION_BYTES.saturating_sub(2))
        );
        let exact_value = ProposedCanonicalValue::from_json(&exact_json).expect("exact value");
        let exact = ConfigurationRevision::new(
            key('7'),
            1,
            None,
            AppendRequestId::new("configured/exact-budget").expect("append id"),
            contract(),
            &exact_value,
        )
        .expect("exact configuration budget is accepted");
        assert_eq!(
            exact.canonical_value.len(),
            MAX_CONFIGURATION_REVISION_BYTES
        );

        let over_json = format!(
            "\"{}\"",
            "x".repeat(MAX_CONFIGURATION_REVISION_BYTES.saturating_sub(1))
        );
        let over_value = ProposedCanonicalValue::from_json(&over_json).expect("over value parses");
        assert!(
            ConfigurationRevision::new(
                key('8'),
                1,
                None,
                AppendRequestId::new("configured/over-budget").expect("append id"),
                contract(),
                &over_value,
            )
            .is_err(),
            "one byte over the generated configuration budget is rejected"
        );
    }

    #[tokio::test]
    async fn revisions_are_append_only_and_resolve_the_current_exact_contract() {
        let store = qualify_and_open_configuration_history(MemoryConfigurationHistoryBackend::new(
            store_scope(),
        ))
        .await
        .expect("semantic open");
        let (writer, reader) = store.into_authorities();
        let stream = key('2');
        let first = writer
            .append(ConfigurationAppendRequest::new(
                stream.clone(),
                None,
                AppendRequestId::new("configured/first").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"revision":1}"#).expect("value"),
            ))
            .await
            .expect("first revision");
        let second = writer
            .append(ConfigurationAppendRequest::new(
                stream.clone(),
                Some(first.content_ref().clone()),
                AppendRequestId::new("configured/second").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"revision":2}"#).expect("value"),
            ))
            .await
            .expect("second revision");
        assert_eq!(second.revision().sequence(), 2);
        assert_eq!(
            second.revision().predecessor_ref(),
            Some(first.content_ref())
        );
        let resolved = reader
            .resolve(&stream, &contract())
            .await
            .expect("current revision");
        assert_eq!(resolved.revision_object(), &second);
        assert_eq!(resolved.canonical_value(), r#"{"revision":2}"#);

        let stale = writer
            .append(ConfigurationAppendRequest::new(
                stream,
                Some(first.content_ref().clone()),
                AppendRequestId::new("configured/stale").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"revision":3}"#).expect("value"),
            ))
            .await;
        assert_eq!(stale, Err(StructuredStoreError::StaleHead));
    }

    #[tokio::test]
    async fn writer_retries_identical_append_after_unknown_acknowledgement() {
        let inner = MemoryConfigurationHistoryBackend::new(store_scope());
        let flaky = flaky_backend(inner, 0, true);
        let (writer, reader) = qualify_and_open_configuration_history(flaky.clone())
            .await
            .expect("semantic open")
            .into_authorities();
        let stream = key('0');
        let revision = writer
            .append(ConfigurationAppendRequest::new(
                stream.clone(),
                None,
                AppendRequestId::new("configured-ack-unknown").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"acknowledged":true}"#).expect("value"),
            ))
            .await
            .expect("identical append retry resolves acknowledgement ambiguity");

        assert_eq!(
            reader
                .resolve(&stream, &contract())
                .await
                .expect("resolve retried append")
                .revision_object(),
            &revision
        );
        let state = flaky.inner.state.lock().expect("memory state");
        assert_eq!(
            state.revisions.get(&stream).map(Vec::len),
            Some(1),
            "ambiguous acknowledgement recovery must not duplicate the durable row"
        );
    }

    #[tokio::test]
    async fn tenant_and_store_keys_do_not_alias() {
        let store = qualify_and_open_configuration_history(MemoryConfigurationHistoryBackend::new(
            store_scope(),
        ))
        .await
        .expect("semantic open");
        let (writer, reader) = store.into_authorities();
        let first_tenant = key('3');
        writer
            .append(ConfigurationAppendRequest::new(
                first_tenant.clone(),
                None,
                AppendRequestId::new("configured/tenant").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"tenant":3}"#).expect("value"),
            ))
            .await
            .expect("tenant revision");
        assert_eq!(
            reader.resolve(&key('4'), &contract()).await,
            Err(StructuredStoreError::RunNotFound)
        );
        let foreign_store = ConfigurationStreamKey::new(
            StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "9".repeat(32)))
                .expect("foreign store"),
            tenant('3'),
            first_tenant.entry_point_operation_id().clone(),
            first_tenant.target_id().clone(),
        );
        assert!(matches!(
            reader.resolve(&foreign_store, &contract()).await,
            Err(StructuredStoreError::InvalidHistory)
        ));
    }

    #[tokio::test]
    async fn operation_and_target_keys_do_not_alias() {
        let store = qualify_and_open_configuration_history(MemoryConfigurationHistoryBackend::new(
            store_scope(),
        ))
        .await
        .expect("semantic open");
        let (writer, reader) = store.into_authorities();
        let selected = key('5');
        writer
            .append(ConfigurationAppendRequest::new(
                selected.clone(),
                None,
                AppendRequestId::new("configured/route").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"route":5}"#).expect("value"),
            ))
            .await
            .expect("route revision");

        let foreign_operation = ConfigurationStreamKey::new(
            store_scope(),
            selected.tenant_scope_id().clone(),
            StableId::new("mfm.fixture/foreign-operation").expect("foreign operation"),
            selected.target_id().clone(),
        );
        let foreign_target = ConfigurationStreamKey::new(
            store_scope(),
            selected.tenant_scope_id().clone(),
            selected.entry_point_operation_id().clone(),
            StableId::new("mfm.fixture/foreign-target").expect("foreign target"),
        );
        assert_eq!(
            reader.resolve(&foreign_operation, &contract()).await,
            Err(StructuredStoreError::RunNotFound)
        );
        assert_eq!(
            reader.resolve(&foreign_target, &contract()).await,
            Err(StructuredStoreError::RunNotFound)
        );
    }

    #[tokio::test]
    async fn independent_head_detects_row_mutation_deletion_truncation_and_rewind() {
        for attack in ["remove-head", "truncate", "rewind-head", "mutate", "delete"] {
            let (backend, stream, first) = two_revision_backend().await;
            {
                let mut state = backend.state.lock().expect("memory state");
                match attack {
                    "remove-head" => {
                        state.heads.remove(&stream);
                    }
                    "truncate" => {
                        state.revisions.get_mut(&stream).expect("revisions").pop();
                    }
                    "rewind-head" => {
                        state.heads.insert(
                            stream.clone(),
                            ConfigurationHistoryHead::new(
                                first.revision().sequence(),
                                first.content_ref().clone(),
                            ),
                        );
                    }
                    "mutate" => {
                        state.revisions.get_mut(&stream).expect("revisions")[0]
                            .decoded
                            .canonical_value = r#"{"revision":9}"#.to_owned();
                    }
                    "delete" => {
                        state
                            .revisions
                            .get_mut(&stream)
                            .expect("revisions")
                            .remove(0);
                    }
                    _ => unreachable!("closed attack table"),
                }
            }
            assert!(
                matches!(
                    qualify_and_open_configuration_history(backend).await,
                    Err(StructuredStoreError::InvalidHistory)
                ),
                "attack {attack} must fail during semantic open"
            );
        }
    }

    async fn two_revision_backend() -> (
        MemoryConfigurationHistoryBackend,
        ConfigurationStreamKey,
        ConfigurationRevisionObject,
    ) {
        let backend = MemoryConfigurationHistoryBackend::new(store_scope());
        let store = qualify_and_open_configuration_history(backend.clone())
            .await
            .expect("semantic open");
        let (writer, _reader) = store.into_authorities();
        let stream = key('6');
        let first = writer
            .append(ConfigurationAppendRequest::new(
                stream.clone(),
                None,
                AppendRequestId::new("configured/tamper-first").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"revision":1}"#).expect("value"),
            ))
            .await
            .expect("first revision");
        writer
            .append(ConfigurationAppendRequest::new(
                stream.clone(),
                Some(first.content_ref().clone()),
                AppendRequestId::new("configured/tamper-second").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"revision":2}"#).expect("value"),
            ))
            .await
            .expect("second revision");
        (backend, stream, first)
    }
}
