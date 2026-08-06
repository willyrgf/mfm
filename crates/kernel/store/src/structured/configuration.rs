//! Append-only configured-value history outside per-run history families.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mfm_canonical::limits::MAX_CONFIGURATION_REVISION_BYTES;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId, StoreScopeId,
    TenantScopeId,
};
use serde::{Deserialize, Serialize};

use super::canonical_append::CanonicalConfigurationAppend;
use super::{ProposedCanonicalValue, StructuredStoreError};

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

#[derive(Serialize)]
struct RevisionPreimage<'a> {
    key: &'a ConfigurationStreamKey,
    sequence: u64,
    predecessor_ref: &'a Option<ContentRef>,
    append_request_id: &'a AppendRequestId,
    value_contract_ref: &'a ContentRef,
    value_ref: &'a ContentRef,
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
    revision_ref: ContentRef,
}

impl ConfigurationRevision {
    fn new(
        key: ConfigurationStreamKey,
        sequence: u64,
        predecessor_ref: Option<ContentRef>,
        append_request_id: AppendRequestId,
        value_contract_ref: ContentRef,
        value: &ProposedCanonicalValue,
    ) -> Result<Self, StructuredStoreError> {
        let canonical_value = value.canonical().as_str().to_owned();
        let value_ref = content_ref(
            "mfm.structured-configured-value",
            canonical_value.as_bytes(),
        )?;
        let revision_ref = revision_ref(
            &key,
            sequence,
            &predecessor_ref,
            &append_request_id,
            &value_contract_ref,
            &value_ref,
        )?;
        let revision = Self {
            key,
            sequence,
            predecessor_ref,
            append_request_id,
            value_contract_ref,
            value_ref,
            canonical_value,
            revision_ref,
        };
        revision.validate()?;
        Ok(revision)
    }

    fn validate(&self) -> Result<(), StructuredStoreError> {
        if self.sequence == 0 {
            return Err(invalid("configuration revision sequence is zero"));
        }
        if self.canonical_value.len() > MAX_CONFIGURATION_REVISION_BYTES {
            return Err(invalid("configuration revision exceeds its byte bound"));
        }
        let canonical =
            PlainCanonicalJsonBytes::from_canonical_json_slice(self.canonical_value.as_bytes())
                .map_err(|_| invalid("configuration value is not canonical JSON"))?;
        if content_ref("mfm.structured-configured-value", canonical.as_bytes())? != self.value_ref
            || revision_ref(
                &self.key,
                self.sequence,
                &self.predecessor_ref,
                &self.append_request_id,
                &self.value_contract_ref,
                &self.value_ref,
            )? != self.revision_ref
        {
            return Err(invalid("configuration revision content address differs"));
        }
        Ok(())
    }

    pub(crate) fn validate_for_ingress(&self) -> Result<(), StructuredStoreError> {
        self.validate()
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

    /// Returns the revision content address.
    pub const fn revision_ref(&self) -> &ContentRef {
        &self.revision_ref
    }
}

/// Store-verified current configured value selected for admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedConfiguredValue {
    revision: ConfigurationRevision,
}

impl VerifiedConfiguredValue {
    /// Returns the immutable selected revision.
    pub const fn revision(&self) -> &ConfigurationRevision {
        &self.revision
    }

    /// Returns exact canonical value bytes.
    pub fn canonical_value(&self) -> &str {
        self.revision.canonical_value()
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
    pub revisions: Vec<ConfigurationRevision>,
}

/// Independently persisted authoritative head of one configured-value stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationHistoryHead {
    sequence: u64,
    revision_ref: ContentRef,
}

impl ConfigurationHistoryHead {
    /// Constructs one physical head pointer loaded by a qualified backend.
    #[doc(hidden)]
    pub const fn new(sequence: u64, revision_ref: ContentRef) -> Self {
        Self {
            sequence,
            revision_ref,
        }
    }

    /// Returns the one-based sequence named by this head.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the exact revision named by this head.
    pub const fn revision_ref(&self) -> &ContentRef {
        &self.revision_ref
    }
}

/// Result of one exact-predecessor configured-value transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationBackendAppendOutcome {
    /// The exact revision was newly committed.
    NewlyCommitted(ConfigurationRevision),
    /// The append identity already names the same revision.
    ExistingSame(ConfigurationRevision),
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

    /// Atomically appends at the exact predecessor.
    fn append<'a>(
        &'a self,
        revision: CanonicalConfigurationAppend,
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

/// Non-cloneable deployment authority for configured-value appends.
pub struct ConfigurationHistoryWriter<B: ConfigurationHistoryBackend> {
    backend: Arc<B>,
}

const MAX_CONFIGURATION_APPEND_LOAD_ATTEMPTS: usize = 8;

async fn load_with_checkpoint_retry<B: ConfigurationHistoryBackend>(
    backend: &B,
    key: &ConfigurationStreamKey,
) -> Result<Option<RawConfigurationHistory>, StructuredStoreError> {
    // SQL commits precede external checkpoint acknowledgement. A concurrent
    // PostgreSQL read can therefore observe one durable prefix before its
    // checkpoint successor; retry only that bounded InvalidHistory window.
    let mut loaded = backend.load(key).await;
    for _ in 1..MAX_CONFIGURATION_APPEND_LOAD_ATTEMPTS {
        if !matches!(&loaded, Err(StructuredStoreError::InvalidHistory)) {
            return loaded;
        }
        loaded = backend.load(key).await;
    }
    loaded
}

impl<B: ConfigurationHistoryBackend> ConfigurationHistoryWriter<B> {
    async fn load_for_append(
        &self,
        key: &ConfigurationStreamKey,
    ) -> Result<Option<RawConfigurationHistory>, StructuredStoreError> {
        load_with_checkpoint_retry(self.backend.as_ref(), key).await
    }

    async fn retry_ambiguous_append(
        &self,
        revision: &ConfigurationRevision,
    ) -> Result<ConfigurationRevision, StructuredStoreError> {
        for _ in 0..MAX_CONFIGURATION_APPEND_LOAD_ATTEMPTS {
            let outcome = self
                .backend
                .append(CanonicalConfigurationAppend::from_store_verified(
                    revision.clone(),
                )?)
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
                    return Err(invalid("configuration backend substituted committed bytes"));
                }
                ConfigurationBackendAppendOutcome::AcknowledgementUnknown => {}
                ConfigurationBackendAppendOutcome::StaleHead => {
                    let resolved = self.load_for_append(revision.key()).await?;
                    match resolved.as_ref().and_then(|history| {
                        history_append_identity(
                            history.revisions.iter(),
                            revision.append_request_id(),
                        )
                    }) {
                        Some(existing) if existing == revision => return Ok(revision.clone()),
                        Some(_) => return Err(StructuredStoreError::AppendConflict),
                        None if resolved.as_ref().is_none_or(|history| {
                            history
                                .head
                                .as_ref()
                                .map(ConfigurationHistoryHead::revision_ref)
                                == revision.predecessor_ref()
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
    ) -> Result<ConfigurationRevision, StructuredStoreError> {
        require_store(self.backend.as_ref(), &request.key)?;
        let history = self.load_for_append(&request.key).await?;
        let verified = history
            .clone()
            .map(verify_configuration_history)
            .transpose()?;
        let current = verified.as_ref().map(VerifiedConfiguredValue::revision);
        if let Some(existing) = history.as_ref().and_then(|history| {
            history_append_identity(history.revisions.iter(), &request.append_request_id)
        }) {
            return if existing.value_contract_ref == request.value_contract_ref
                && existing.canonical_value == request.value.canonical().as_str()
            {
                Ok(existing.clone())
            } else {
                Err(StructuredStoreError::AppendConflict)
            };
        }
        if current.map(ConfigurationRevision::revision_ref)
            != request.expected_predecessor_ref.as_ref()
        {
            return Err(StructuredStoreError::StaleHead);
        }
        let sequence = current.map_or(1, |revision| revision.sequence + 1);
        let revision = ConfigurationRevision::new(
            request.key,
            sequence,
            request.expected_predecessor_ref,
            request.append_request_id,
            request.value_contract_ref,
            &request.value,
        )?;
        match self
            .backend
            .append(CanonicalConfigurationAppend::from_store_verified(
                revision.clone(),
            )?)
            .await?
        {
            ConfigurationBackendAppendOutcome::NewlyCommitted(returned)
            | ConfigurationBackendAppendOutcome::ExistingSame(returned)
                if returned == revision =>
            {
                Ok(revision)
            }
            ConfigurationBackendAppendOutcome::NewlyCommitted(_)
            | ConfigurationBackendAppendOutcome::ExistingSame(_) => {
                Err(invalid("configuration backend substituted committed bytes"))
            }
            ConfigurationBackendAppendOutcome::StaleHead => {
                // A concurrent append with the same logical identity can win
                // after this writer's initial read but before the backend lock.
                // Reclassify the unchanged identity from a fresh read so a
                // race cannot turn an append conflict into an unrelated stale
                // predecessor result.
                for _ in 0..MAX_CONFIGURATION_APPEND_LOAD_ATTEMPTS {
                    let resolved = self.load_for_append(revision.key()).await?;
                    match resolved.as_ref().and_then(|history| {
                        history_append_identity(
                            history.revisions.iter(),
                            revision.append_request_id(),
                        )
                    }) {
                        Some(existing) if existing == &revision => return Ok(revision),
                        Some(_) => return Err(StructuredStoreError::AppendConflict),
                        None => {
                            match self
                                .backend
                                .append(CanonicalConfigurationAppend::from_store_verified(
                                    revision.clone(),
                                )?)
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
                                    return Err(invalid(
                                        "configuration backend substituted committed bytes",
                                    ));
                                }
                                ConfigurationBackendAppendOutcome::StaleHead => {}
                                ConfigurationBackendAppendOutcome::AcknowledgementUnknown => {}
                            }
                        }
                    }
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
        let history = load_with_checkpoint_retry(self.backend.as_ref(), key)
            .await?
            .ok_or(StructuredStoreError::RunNotFound)?;
        let verified = verify_configuration_history(history)?;
        if verified.revision.value_contract_ref != *expected_value_contract_ref {
            return Err(invalid("configured value contract differs"));
        }
        Ok(verified)
    }
}

/// One-shot configured-history assembly separating append and resolve authority.
pub struct ConfigurationHistoryStore<B: ConfigurationHistoryBackend> {
    backend: Arc<B>,
}

impl<B: ConfigurationHistoryBackend> ConfigurationHistoryStore<B> {
    /// Binds one qualified raw backend.
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }

    /// Separates deployment append authority from application resolve authority.
    pub fn split(self) -> (ConfigurationHistoryWriter<B>, ConfigurationHistoryReader<B>) {
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

/// In-memory conformance backend for configured-value history.
#[derive(Clone)]
pub struct MemoryConfigurationHistoryBackend {
    store_scope_id: StoreScopeId,
    state: Arc<Mutex<MemoryConfigurationHistoryState>>,
}

#[derive(Default)]
struct MemoryConfigurationHistoryState {
    revisions: BTreeMap<ConfigurationStreamKey, Vec<ConfigurationRevision>>,
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

    fn append<'a>(
        &'a self,
        revision: CanonicalConfigurationAppend,
    ) -> ConfigurationBackendFuture<'a, ConfigurationBackendAppendOutcome> {
        Box::pin(async move {
            if !revision.is_store_verified() {
                return Err(StructuredStoreError::InvalidHistory);
            }
            let revision = revision.into_revision();
            let mut state = self
                .state
                .lock()
                .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let history = state
                .revisions
                .get(&revision.key)
                .cloned()
                .unwrap_or_default();
            let head = state.heads.get(&revision.key).cloned();
            if history.is_empty() != head.is_none() {
                return Err(invalid("configuration revisions and head disagree"));
            }
            if !history.is_empty() {
                verify_configuration_history(RawConfigurationHistory {
                    key: revision.key.clone(),
                    head: head.clone(),
                    revisions: history.clone(),
                })?;
            }
            if let Some(existing) = history
                .iter()
                .find(|existing| existing.append_request_id == revision.append_request_id)
            {
                return if existing == &revision {
                    Ok(ConfigurationBackendAppendOutcome::ExistingSame(
                        existing.clone(),
                    ))
                } else {
                    Err(StructuredStoreError::AppendConflict)
                };
            }
            if head.as_ref().map(ConfigurationHistoryHead::revision_ref)
                != revision.predecessor_ref.as_ref()
                || head
                    .as_ref()
                    .map_or(1, |head| head.sequence().saturating_add(1))
                    != revision.sequence
            {
                return Ok(ConfigurationBackendAppendOutcome::StaleHead);
            }
            state
                .revisions
                .entry(revision.key.clone())
                .or_default()
                .push(revision.clone());
            state.heads.insert(
                revision.key.clone(),
                ConfigurationHistoryHead::new(revision.sequence, revision.revision_ref.clone()),
            );
            Ok(ConfigurationBackendAppendOutcome::NewlyCommitted(revision))
        })
    }
}

/// Callback-free validation shared with qualified physical backends before mutation.
#[doc(hidden)]
pub fn verify_configuration_history(
    history: RawConfigurationHistory,
) -> Result<VerifiedConfiguredValue, StructuredStoreError> {
    let head = history
        .head
        .as_ref()
        .ok_or_else(|| invalid("configuration history head is absent"))?;
    if history.revisions.is_empty() {
        return Err(invalid("configuration history revisions are absent"));
    }
    let mut predecessor = None;
    let mut append_ids = BTreeSet::new();
    let mut expected_contract = None;
    for (index, revision) in history.revisions.iter().enumerate() {
        revision.validate()?;
        if revision.key != history.key
            || revision.sequence
                != u64::try_from(index + 1)
                    .map_err(|_| invalid("configuration history sequence cannot be represented"))?
            || revision.predecessor_ref != predecessor
            || !append_ids.insert(revision.append_request_id.clone())
            || expected_contract.get_or_insert_with(|| revision.value_contract_ref.clone())
                != &revision.value_contract_ref
        {
            return Err(invalid("configuration history prefix is invalid"));
        }
        predecessor = Some(revision.revision_ref.clone());
    }
    let revision = history
        .revisions
        .last()
        .cloned()
        .ok_or_else(|| invalid("configuration history is empty"))?;
    if history.revisions.len()
        != usize::try_from(head.sequence)
            .map_err(|_| invalid("configuration head sequence cannot be represented"))?
        || revision.sequence != head.sequence
        || revision.revision_ref != head.revision_ref
    {
        return Err(invalid(
            "configuration history head differs from its exact prefix",
        ));
    }
    Ok(VerifiedConfiguredValue { revision })
}

fn history_append_identity<'a>(
    revisions: impl Iterator<Item = &'a ConfigurationRevision>,
    append_request_id: &AppendRequestId,
) -> Option<&'a ConfigurationRevision> {
    revisions
        .into_iter()
        .find(|revision| revision.append_request_id == *append_request_id)
}

fn require_store(
    backend: &impl ConfigurationHistoryBackend,
    key: &ConfigurationStreamKey,
) -> Result<(), StructuredStoreError> {
    if key.store_scope_id != *backend.store_scope_id() {
        return Err(invalid("configuration stream belongs to another store"));
    }
    Ok(())
}

fn revision_ref(
    key: &ConfigurationStreamKey,
    sequence: u64,
    predecessor_ref: &Option<ContentRef>,
    append_request_id: &AppendRequestId,
    value_contract_ref: &ContentRef,
    value_ref: &ContentRef,
) -> Result<ContentRef, StructuredStoreError> {
    let canonical = mfm_journal::structured::canonical_json(&RevisionPreimage {
        key,
        sequence,
        predecessor_ref,
        append_request_id,
        value_contract_ref,
        value_ref,
    })
    .map_err(|_| invalid("configuration revision cannot be canonicalized"))?;
    content_ref(
        "mfm.structured-configuration-revision",
        canonical.as_bytes(),
    )
}

fn content_ref(name: &str, bytes: &[u8]) -> Result<ContentRef, StructuredStoreError> {
    let schema_id = SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.structured-schema.v1:{name}:1").as_bytes()),
    )
    .map_err(|_| invalid("configuration schema identity is invalid"))?;
    ContentRef::new(
        schema_id,
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(bytes)),
    )
    .map_err(|_| invalid("configuration content reference is invalid"))
}

fn invalid(_message: &'static str) -> StructuredStoreError {
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
        content_ref("mfm.fixture-configured-contract", b"contract").expect("contract")
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

        fn append<'a>(
            &'a self,
            revision: CanonicalConfigurationAppend,
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
        let store =
            ConfigurationHistoryStore::new(MemoryConfigurationHistoryBackend::new(store_scope()));
        let (writer, reader) = store.split();
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
                Some(first.revision_ref().clone()),
                AppendRequestId::new("configured/second").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"revision":2}"#).expect("value"),
            ))
            .await
            .expect("second revision");
        assert_eq!(second.sequence(), 2);
        assert_eq!(second.predecessor_ref(), Some(first.revision_ref()));
        let resolved = reader
            .resolve(&stream, &contract())
            .await
            .expect("current revision");
        assert_eq!(resolved.revision(), &second);
        assert_eq!(resolved.canonical_value(), r#"{"revision":2}"#);

        let stale = writer
            .append(ConfigurationAppendRequest::new(
                stream,
                Some(first.revision_ref().clone()),
                AppendRequestId::new("configured/stale").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"revision":3}"#).expect("value"),
            ))
            .await;
        assert_eq!(stale, Err(StructuredStoreError::StaleHead));
    }

    #[tokio::test]
    async fn reader_retries_bounded_transient_checkpoint_mismatch() {
        let inner = MemoryConfigurationHistoryBackend::new(store_scope());
        let stream = key('9');
        let (writer, _) = ConfigurationHistoryStore::new(inner.clone()).split();
        let expected = writer
            .append(ConfigurationAppendRequest::new(
                stream.clone(),
                None,
                AppendRequestId::new("configured-reader-retry").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"retry":true}"#).expect("value"),
            ))
            .await
            .expect("revision");

        let flaky = flaky_backend(inner, 2, false);
        let (_, reader) = ConfigurationHistoryStore::new(flaky.clone()).split();
        assert_eq!(
            reader
                .resolve(&stream, &contract())
                .await
                .expect("reader retries transient mismatch")
                .revision(),
            &expected
        );
        assert_eq!(
            flaky.transient_loads.load(Ordering::Acquire),
            0,
            "all transient load failures must be consumed by the bounded retry"
        );
    }

    #[tokio::test]
    async fn writer_retries_identical_append_after_unknown_acknowledgement() {
        let inner = MemoryConfigurationHistoryBackend::new(store_scope());
        let flaky = flaky_backend(inner, 0, true);
        let (writer, reader) = ConfigurationHistoryStore::new(flaky.clone()).split();
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
                .revision(),
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
        let store =
            ConfigurationHistoryStore::new(MemoryConfigurationHistoryBackend::new(store_scope()));
        let (writer, reader) = store.split();
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
        let store =
            ConfigurationHistoryStore::new(MemoryConfigurationHistoryBackend::new(store_scope()));
        let (writer, reader) = store.split();
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
                                first.sequence(),
                                first.revision_ref().clone(),
                            ),
                        );
                    }
                    "mutate" => {
                        state.revisions.get_mut(&stream).expect("revisions")[0].canonical_value =
                            r#"{"revision":9}"#.to_owned();
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
            let (_writer, reader) = ConfigurationHistoryStore::new(backend).split();
            assert_eq!(
                reader.resolve(&stream, &contract()).await,
                Err(StructuredStoreError::InvalidHistory),
                "attack {attack} must fail closed"
            );
        }
    }

    async fn two_revision_backend() -> (
        MemoryConfigurationHistoryBackend,
        ConfigurationStreamKey,
        ConfigurationRevision,
    ) {
        let backend = MemoryConfigurationHistoryBackend::new(store_scope());
        let store = ConfigurationHistoryStore::new(backend.clone());
        let (writer, _reader) = store.split();
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
                Some(first.revision_ref().clone()),
                AppendRequestId::new("configured/tamper-second").expect("append id"),
                contract(),
                ProposedCanonicalValue::from_json(r#"{"revision":2}"#).expect("value"),
            ))
            .await
            .expect("second revision");
        (backend, stream, first)
    }
}
