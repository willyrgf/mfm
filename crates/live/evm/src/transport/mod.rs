//! Bounded exact-generation EVM JSON-RPC transport.
//!
//! Construction and routing selection perform no provider IO. Every public
//! operation method performs zero or one HTTP exchange against the exact
//! immutable generation carried by its typed request. Redirects, retries,
//! failover, current-route aliases, batching, and arbitrary JSON-RPC calls are
//! absent.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use alloy_primitives::{Address, B256, U256};
use bytes::Bytes;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_evm::{
    EvmAnchorConfirmationRequest, EvmAnchoredSource, EvmBlockAnchor, EvmBlockResponse,
    EvmChainIdentityRequest, EvmChainIdentityResponse, EvmCheckedSource, EvmCoarseSizeClass,
    EvmLatestAnchorRequest, EvmNativeBalanceRequest, EvmQuantityResponse, EvmResponseInvalidKind,
    EvmRoutingGenerationRef, EvmSafeFailure, EvmTokenBalanceRequest, EvmTokenDecimalsRequest,
    EvmTokenDecimalsResponse, EvmWalletObservedTransaction, EvmWalletReceipt,
    TransientSignedEip1559Envelope,
};
use mfm_ids::{ContentRef, DigestAlgorithm, LocalPublicId, SchemaId, SemanticTypeId, StableId};
use mfm_program::boundary_content_ref;
use mfm_values::RetainedValueContract;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::debug;
use zeroize::{Zeroize, Zeroizing};

mod exact;

pub(crate) use exact::WalletBroadcastResponse;
use exact::{decode_response, DecodeFailure, EncodedRpcRequest, ExactRpcRequest, ExactRpcResponse};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_AUTHORIZATION_BYTES: usize = 16 * 1024;
const MAX_GLOBAL_IN_FLIGHT_EXCHANGES: usize = 64;
const MAX_IN_FLIGHT_EXCHANGES_PER_GENERATION: usize = 16;
const MAX_IDLE_CONNECTIONS_PER_HOST: usize = 16;
const JSON_RPC_IMPLEMENTATION_ID: &str = "mfm.evm.json-rpc.v1";
pub(crate) const EXACT_ALREADY_KNOWN_CODE: i64 = -32_000;
pub(crate) const EXACT_ALREADY_KNOWN_MESSAGE: &str = "already known";
pub(crate) const EVM_SEND_RAW_TRANSACTION_METHOD: &str = "eth_sendRawTransaction";
pub(crate) const EVM_TRANSACTION_BY_HASH_METHOD: &str = "eth_getTransactionByHash";
pub(crate) const EVM_RECEIPT_BY_HASH_METHOD: &str = "eth_getTransactionReceipt";
pub(crate) const EVM_BLOCK_BY_NUMBER_METHOD: &str = "eth_getBlockByNumber";

#[cfg(test)]
tokio::task_local! {
    static EXCHANGE_OWNER_PROBE: Arc<ExchangeOwnerProbe>;
}

#[cfg(test)]
async fn with_exchange_owner_probe<Future>(
    probe: Arc<ExchangeOwnerProbe>,
    future: Future,
) -> Future::Output
where
    Future: std::future::Future,
{
    EXCHANGE_OWNER_PROBE.scope(probe, future).await
}

#[cfg(test)]
fn current_exchange_owner_probe() -> Option<Arc<ExchangeOwnerProbe>> {
    EXCHANGE_OWNER_PROBE.try_with(Arc::clone).ok()
}

#[cfg(test)]
#[derive(Default)]
struct ExchangeOwnerProbe {
    request_created: AtomicUsize,
    request_pointer: AtomicUsize,
    request_length: AtomicUsize,
    request_dropped: AtomicUsize,
    request_zeroized: AtomicUsize,
    response_created: AtomicUsize,
    response_capacity: AtomicUsize,
    response_dropped: AtomicUsize,
    response_zeroized: AtomicUsize,
}
/// Fixed provider class reviewed by the EVM JSON-RPC adapter.
pub const EVM_JSON_RPC_PROVIDER_CLASS: &str = "mfm.evm-json-rpc";
/// Fixed route policy: one endpoint entry, no retry, redirect, or fallback.
pub const EVM_ROUTE_POLICY_ID: &str = "mfm.evm.single-entry-no-retry";
/// Version of the fixed route policy.
pub const EVM_ROUTE_POLICY_VERSION: &str = "1";
/// Exact routing-generation descriptor version.
pub const EVM_ROUTING_GENERATION_DESCRIPTOR_VERSION: &str = "mfm.evm.routing-generation.v1";
/// Exact aggregate routing-catalog descriptor version.
pub const EVM_ROUTING_CATALOG_DESCRIPTOR_VERSION: &str = "mfm.evm.routing-catalog.v1";

/// Result type for local transport and routing construction.
pub type TransportResult<T> = std::result::Result<T, EvmTransportError>;

/// Transient result of one EVM read before safe-failure classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmTransportOutcome<R> {
    /// The destination returned one typed response.
    Returned(R),
    /// Boundary entry was proven not to have occurred.
    DidNotEnter(EvmSafeFailure),
    /// Boundary entry or the terminal outcome remains indeterminate.
    Indeterminate(EvmSafeFailure),
}

/// Redaction-safe local setup failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmTransportError {
    /// Endpoint, authorization, source identity, or HTTP client setup was invalid.
    #[error("EVM transport configuration is invalid")]
    InvalidConfiguration,
    /// The same immutable routing generation was registered twice.
    #[error("EVM routing generation is duplicated")]
    DuplicateGeneration,
    /// No immutable generation was supplied for a qualified catalog.
    #[error("EVM routing catalog is empty")]
    EmptyCatalog,
}

/// Checked resolved HTTP(S) endpoint.
#[derive(Clone)]
pub struct EvmRpcEndpoint {
    url: reqwest::Url,
}

impl EvmRpcEndpoint {
    /// Admits an endpoint without embedded credentials, query, or fragment.
    pub fn new(endpoint: impl AsRef<str>) -> TransportResult<Self> {
        let url = reqwest::Url::parse(endpoint.as_ref())
            .map_err(|_| EvmTransportError::InvalidConfiguration)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(EvmTransportError::InvalidConfiguration);
        }
        Ok(Self { url })
    }
}

impl fmt::Debug for EvmRpcEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EvmRpcEndpoint(<redacted>)")
    }
}

/// Consumed resolved HTTP authorization value.
pub struct EvmRpcAuthorization {
    value: HeaderValue,
}

impl EvmRpcAuthorization {
    /// Admits one non-empty valid HTTP authorization header of at most 16 KiB.
    pub fn new(value: Zeroizing<String>) -> TransportResult<Self> {
        if value.is_empty() || value.len() > MAX_AUTHORIZATION_BYTES {
            return Err(EvmTransportError::InvalidConfiguration);
        }
        let mut value = value;
        let bytes = Zeroizing::new(std::mem::take(&mut *value).into_bytes());
        let owner = ZeroizingBytesOwner::new(bytes);
        let mut value = HeaderValue::from_maybe_shared(Bytes::from_owner(owner))
            .map_err(|_| EvmTransportError::InvalidConfiguration)?;
        value.set_sensitive(true);
        Ok(Self { value })
    }
}

struct ZeroizingBytesOwner {
    bytes: Zeroizing<Vec<u8>>,
    #[cfg(test)]
    drop_probe: Option<Arc<ZeroizingOwnerDropProbe>>,
    #[cfg(test)]
    exchange_probe: Option<Arc<ExchangeOwnerProbe>>,
}

impl ZeroizingBytesOwner {
    fn new(bytes: Zeroizing<Vec<u8>>) -> Self {
        #[cfg(test)]
        let exchange_probe = current_exchange_owner_probe();
        #[cfg(test)]
        if let Some(probe) = &exchange_probe {
            probe.request_created.fetch_add(1, Ordering::SeqCst);
            probe
                .request_pointer
                .store(bytes.as_ptr() as usize, Ordering::SeqCst);
            probe.request_length.store(bytes.len(), Ordering::SeqCst);
        }
        Self {
            bytes,
            #[cfg(test)]
            drop_probe: None,
            #[cfg(test)]
            exchange_probe,
        }
    }

    #[cfg(test)]
    fn with_probe(bytes: Zeroizing<Vec<u8>>, drop_probe: Arc<ZeroizingOwnerDropProbe>) -> Self {
        Self {
            bytes,
            drop_probe: Some(drop_probe),
            exchange_probe: None,
        }
    }
}

impl AsRef<[u8]> for ZeroizingBytesOwner {
    fn as_ref(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

fn request_body(body: Zeroizing<Vec<u8>>) -> reqwest::Body {
    request_body_from_owner(ZeroizingBytesOwner::new(body))
}

fn request_body_from_owner(owner: ZeroizingBytesOwner) -> reqwest::Body {
    reqwest::Body::from(Bytes::from_owner(owner))
}

impl Drop for ZeroizingBytesOwner {
    fn drop(&mut self) {
        self.bytes.as_mut_slice().zeroize();
        #[cfg(test)]
        if let Some(probe) = &self.drop_probe {
            probe
                .zeroized
                .store(self.bytes.iter().all(|byte| *byte == 0), Ordering::SeqCst);
            probe.dropped.store(true, Ordering::SeqCst);
        }
        #[cfg(test)]
        if let Some(probe) = &self.exchange_probe {
            probe.request_zeroized.fetch_add(
                usize::from(self.bytes.iter().all(|byte| *byte == 0)),
                Ordering::SeqCst,
            );
            probe.request_dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
#[derive(Default)]
struct ZeroizingOwnerDropProbe {
    dropped: AtomicBool,
    zeroized: AtomicBool,
}

struct ZeroizingResponseBuffer {
    bytes: Zeroizing<Vec<u8>>,
    #[cfg(test)]
    drop_probe: Option<Arc<ZeroizingOwnerDropProbe>>,
    #[cfg(test)]
    exchange_probe: Option<Arc<ExchangeOwnerProbe>>,
}

impl ZeroizingResponseBuffer {
    fn new() -> Self {
        #[cfg(test)]
        let exchange_probe = current_exchange_owner_probe();
        #[cfg(test)]
        if let Some(probe) = &exchange_probe {
            probe.response_created.fetch_add(1, Ordering::SeqCst);
            probe
                .response_capacity
                .store(MAX_RESPONSE_BYTES, Ordering::SeqCst);
        }
        Self {
            bytes: Zeroizing::new(Vec::with_capacity(MAX_RESPONSE_BYTES)),
            #[cfg(test)]
            drop_probe: None,
            #[cfg(test)]
            exchange_probe,
        }
    }

    #[cfg(test)]
    fn with_probe(drop_probe: Arc<ZeroizingOwnerDropProbe>) -> Self {
        Self {
            bytes: Zeroizing::new(Vec::with_capacity(MAX_RESPONSE_BYTES)),
            drop_probe: Some(drop_probe),
            exchange_probe: None,
        }
    }

    fn extend(&mut self, chunk: &[u8]) -> Result<(), usize> {
        let next_len = self
            .bytes
            .len()
            .checked_add(chunk.len())
            .ok_or(usize::MAX)?;
        if next_len > MAX_RESPONSE_BYTES {
            return Err(next_len);
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }
}

impl Drop for ZeroizingResponseBuffer {
    fn drop(&mut self) {
        self.bytes.as_mut_slice().zeroize();
        #[cfg(test)]
        if let Some(probe) = &self.drop_probe {
            probe
                .zeroized
                .store(self.bytes.iter().all(|byte| *byte == 0), Ordering::SeqCst);
            probe.dropped.store(true, Ordering::SeqCst);
        }
        #[cfg(test)]
        if let Some(probe) = &self.exchange_probe {
            probe.response_zeroized.fetch_add(
                usize::from(self.bytes.iter().all(|byte| *byte == 0)),
                Ordering::SeqCst,
            );
            probe.response_dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl fmt::Debug for EvmRpcAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EvmRpcAuthorization(<redacted>)")
    }
}

/// Immutable secret-free identity of one resolved EVM route generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvmRoutingGenerationDescriptor {
    version: String,
    network_id: String,
    source_ref: String,
    chain_id: u64,
    generation_id: StableId,
    provider_class: String,
    route_policy_id: String,
    route_policy_version: String,
}

impl EvmRoutingGenerationDescriptor {
    fn new(
        network_id: impl Into<String>,
        source_ref: impl Into<String>,
        chain_id: u64,
        generation_id: StableId,
    ) -> TransportResult<Self> {
        let network_id = network_id.into();
        let source_ref = source_ref.into();
        LocalPublicId::new(&network_id).map_err(|_| EvmTransportError::InvalidConfiguration)?;
        LocalPublicId::new(&source_ref).map_err(|_| EvmTransportError::InvalidConfiguration)?;
        if chain_id == 0 {
            return Err(EvmTransportError::InvalidConfiguration);
        }
        Ok(Self {
            version: EVM_ROUTING_GENERATION_DESCRIPTOR_VERSION.to_owned(),
            network_id,
            source_ref,
            chain_id,
            generation_id,
            provider_class: EVM_JSON_RPC_PROVIDER_CLASS.to_owned(),
            route_policy_id: EVM_ROUTE_POLICY_ID.to_owned(),
            route_policy_version: EVM_ROUTE_POLICY_VERSION.to_owned(),
        })
    }

    fn validate(&self) -> TransportResult<()> {
        LocalPublicId::new(&self.network_id)
            .map_err(|_| EvmTransportError::InvalidConfiguration)?;
        LocalPublicId::new(&self.source_ref)
            .map_err(|_| EvmTransportError::InvalidConfiguration)?;
        if self.version != EVM_ROUTING_GENERATION_DESCRIPTOR_VERSION
            || self.chain_id == 0
            || self.provider_class != EVM_JSON_RPC_PROVIDER_CLASS
            || self.route_policy_id != EVM_ROUTE_POLICY_ID
            || self.route_policy_version != EVM_ROUTE_POLICY_VERSION
        {
            return Err(EvmTransportError::InvalidConfiguration);
        }
        Ok(())
    }

    /// Returns the exact descriptor version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the semantic network selected by this generation.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the reviewed local source identity.
    pub fn source_ref(&self) -> &str {
        &self.source_ref
    }

    /// Returns the expected non-zero chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the operator-controlled immutable generation identity.
    pub const fn generation_id(&self) -> &StableId {
        &self.generation_id
    }

    /// Returns the fixed reviewed provider class.
    pub fn provider_class(&self) -> &str {
        &self.provider_class
    }

    /// Returns the fixed one-entry/no-retry route policy.
    pub fn route_policy_id(&self) -> &str {
        &self.route_policy_id
    }

    /// Returns the fixed route-policy version.
    pub fn route_policy_version(&self) -> &str {
        &self.route_policy_version
    }

    /// Returns exact canonical descriptor bytes.
    pub fn canonical(&self) -> TransportResult<PlainCanonicalJsonBytes> {
        self.validate()?;
        canonical_json(self)
    }

    /// Returns the exact descriptor content identity used by typed requests.
    pub fn content_ref(&self) -> TransportResult<ContentRef> {
        boundary_content_ref(
            routing_generation_descriptor_schema_id()?,
            &self.canonical()?,
        )
        .map_err(|_| EvmTransportError::InvalidConfiguration)
    }
}

impl<'de> Deserialize<'de> for EvmRoutingGenerationDescriptor {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            version: String,
            network_id: String,
            source_ref: String,
            chain_id: u64,
            generation_id: StableId,
            provider_class: String,
            route_policy_id: String,
            route_policy_version: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let descriptor = Self {
            version: wire.version,
            network_id: wire.network_id,
            source_ref: wire.source_ref,
            chain_id: wire.chain_id,
            generation_id: wire.generation_id,
            provider_class: wire.provider_class,
            route_policy_id: wire.route_policy_id,
            route_policy_version: wire.route_policy_version,
        };
        descriptor.validate().map_err(serde::de::Error::custom)?;
        Ok(descriptor)
    }
}

/// Aggregate immutable catalog identity retained by one read binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvmRoutingCatalogDescriptor {
    version: String,
    ordered_generation_refs: Vec<EvmRoutingGenerationRef>,
}

impl EvmRoutingCatalogDescriptor {
    fn validate(&self) -> TransportResult<()> {
        if self.version != EVM_ROUTING_CATALOG_DESCRIPTOR_VERSION
            || self.ordered_generation_refs.is_empty()
            || self
                .ordered_generation_refs
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(EvmTransportError::InvalidConfiguration);
        }
        Ok(())
    }

    /// Returns the exact descriptor version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns exact generation references in content-reference order.
    pub fn ordered_generation_refs(&self) -> &[EvmRoutingGenerationRef] {
        &self.ordered_generation_refs
    }

    /// Returns exact canonical descriptor bytes.
    pub fn canonical(&self) -> TransportResult<PlainCanonicalJsonBytes> {
        self.validate()?;
        canonical_json(self)
    }

    /// Returns the aggregate catalog content identity.
    pub fn content_ref(&self) -> TransportResult<ContentRef> {
        boundary_content_ref(routing_catalog_descriptor_schema_id()?, &self.canonical()?)
            .map_err(|_| EvmTransportError::InvalidConfiguration)
    }
}

impl<'de> Deserialize<'de> for EvmRoutingCatalogDescriptor {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            version: String,
            ordered_generation_refs: Vec<EvmRoutingGenerationRef>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let descriptor = Self {
            version: wire.version,
            ordered_generation_refs: wire.ordered_generation_refs,
        };
        descriptor.validate().map_err(serde::de::Error::custom)?;
        Ok(descriptor)
    }
}

struct ResolvedGeneration {
    descriptor: EvmRoutingGenerationDescriptor,
    implementation_id: String,
    endpoint: EvmRpcEndpoint,
    authorization: Option<EvmRpcAuthorization>,
    limit: Arc<Semaphore>,
}

/// Builder for immutable, exact-reference routing generations.
#[derive(Default)]
pub struct EvmRoutingCatalogBuilder {
    generations: BTreeMap<EvmRoutingGenerationRef, Arc<ResolvedGeneration>>,
}

impl EvmRoutingCatalogBuilder {
    /// Starts an empty local generation catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts one exact generation descriptor and its private route.
    ///
    /// Endpoint or authorization rotation requires a new operator-selected
    /// `generation_id`; neither secret-bearing value participates in the
    /// persisted descriptor.
    pub fn insert(
        &mut self,
        network_id: impl Into<String>,
        source_ref: impl Into<String>,
        chain_id: u64,
        generation_id: StableId,
        endpoint: EvmRpcEndpoint,
        authorization: Option<EvmRpcAuthorization>,
    ) -> TransportResult<EvmRoutingGenerationRef> {
        let descriptor =
            EvmRoutingGenerationDescriptor::new(network_id, source_ref, chain_id, generation_id)?;
        let generation = EvmRoutingGenerationRef::from_content_ref(descriptor.content_ref()?)
            .map_err(|_| EvmTransportError::InvalidConfiguration)?;
        if self.generations.contains_key(&generation) {
            return Err(EvmTransportError::DuplicateGeneration);
        }
        self.generations.insert(
            generation.clone(),
            Arc::new(ResolvedGeneration {
                descriptor,
                implementation_id: JSON_RPC_IMPLEMENTATION_ID.to_owned(),
                endpoint,
                authorization,
                limit: Arc::new(Semaphore::new(MAX_IN_FLIGHT_EXCHANGES_PER_GENERATION)),
            }),
        );
        Ok(generation)
    }

    /// Seals the exact generation catalog.
    pub fn build(self) -> TransportResult<EvmRoutingCatalog> {
        if self.generations.is_empty() {
            return Err(EvmTransportError::EmptyCatalog);
        }
        let descriptor = EvmRoutingCatalogDescriptor {
            version: EVM_ROUTING_CATALOG_DESCRIPTOR_VERSION.to_owned(),
            ordered_generation_refs: self.generations.keys().cloned().collect(),
        };
        Ok(EvmRoutingCatalog {
            generations: self.generations,
            descriptor,
        })
    }
}

/// Immutable local routing catalog with no alias lookup.
pub struct EvmRoutingCatalog {
    generations: BTreeMap<EvmRoutingGenerationRef, Arc<ResolvedGeneration>>,
    descriptor: EvmRoutingCatalogDescriptor,
}

impl EvmRoutingCatalog {
    fn resolve(&self, generation: &EvmRoutingGenerationRef) -> Option<Arc<ResolvedGeneration>> {
        self.generations.get(generation).cloned()
    }

    /// Returns whether the exact generation is locally available.
    pub fn contains(&self, generation: &EvmRoutingGenerationRef) -> bool {
        self.generations.contains_key(generation)
    }

    /// Returns the number of immutable generations. No aliases are included.
    pub fn len(&self) -> usize {
        self.generations.len()
    }

    /// Returns whether no generation is configured.
    pub fn is_empty(&self) -> bool {
        self.generations.is_empty()
    }

    /// Returns the immutable aggregate catalog descriptor.
    pub const fn descriptor(&self) -> &EvmRoutingCatalogDescriptor {
        &self.descriptor
    }

    /// Returns every secret-free generation descriptor in reference order.
    pub fn generation_descriptors(
        &self,
    ) -> impl Iterator<Item = (&EvmRoutingGenerationRef, &EvmRoutingGenerationDescriptor)> {
        self.generations
            .iter()
            .map(|(reference, generation)| (reference, &generation.descriptor))
    }
}

impl fmt::Debug for EvmRoutingCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvmRoutingCatalog")
            .field("generation_count", &self.generations.len())
            .finish()
    }
}

/// Shared bounded HTTP transport for exact typed EVM operations.
#[derive(Clone)]
pub struct EvmJsonRpcTransport {
    shared: Arc<SharedHttpRuntime>,
    routes: Arc<EvmRoutingCatalog>,
}

struct SharedHttpRuntime {
    client: reqwest::Client,
    global_limit: Arc<Semaphore>,
}

impl EvmJsonRpcTransport {
    /// Constructs the transport without resolving a route or performing IO.
    pub fn new(routes: EvmRoutingCatalog) -> TransportResult<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .pool_max_idle_per_host(MAX_IDLE_CONNECTIONS_PER_HOST)
            .build()
            .map_err(|_| EvmTransportError::InvalidConfiguration)?;
        Ok(Self {
            shared: Arc::new(SharedHttpRuntime {
                client,
                global_limit: Arc::new(Semaphore::new(MAX_GLOBAL_IN_FLIGHT_EXCHANGES)),
            }),
            routes: Arc::new(routes),
        })
    }

    /// Returns the exact secret-free aggregate routing descriptor.
    pub fn routing_catalog_descriptor(&self) -> &EvmRoutingCatalogDescriptor {
        self.routes.descriptor()
    }

    /// Returns every exact secret-free generation descriptor in reference order.
    pub fn routing_generation_descriptors(
        &self,
    ) -> impl Iterator<Item = (&EvmRoutingGenerationRef, &EvmRoutingGenerationDescriptor)> {
        self.routes.generation_descriptors()
    }

    pub(crate) fn is_same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared) && Arc::ptr_eq(&self.routes, &other.routes)
    }

    /// Performs one exact `eth_chainId` operation.
    pub async fn chain_identity(
        &self,
        request: &EvmChainIdentityRequest,
    ) -> EvmTransportOutcome<EvmChainIdentityResponse> {
        let route = match self.resolve_binding(request.binding()) {
            Ok(route) => route,
            Err(failure) => return failure.into_outcome(),
        };
        let chain_id = match self
            .exchange(route.clone(), ExactRpcRequest::ChainIdentity)
            .await
        {
            Ok(ExactRpcResponse::ChainIdentity(chain_id)) => chain_id,
            Ok(_) => {
                return indeterminate(EvmSafeFailure::ResponseInvalid {
                    response_kind: EvmResponseInvalidKind::InvalidResult,
                    size_class: EvmCoarseSizeClass::UpTo16Kib,
                });
            }
            Err(failure) => return failure.into_outcome(),
        };
        EvmTransportOutcome::Returned(EvmChainIdentityResponse {
            chain_id,
            source_scope: route.descriptor.source_ref.clone(),
            implementation_id: route.implementation_id.clone(),
        })
    }

    /// Performs one exact latest `eth_getBlockByNumber` operation.
    pub async fn latest_anchor(
        &self,
        request: &EvmLatestAnchorRequest,
    ) -> EvmTransportOutcome<EvmBlockResponse> {
        let route = match self.resolve_source(request.source()) {
            Ok(route) => route,
            Err(failure) => return failure.into_outcome(),
        };
        match self.exchange(route, ExactRpcRequest::LatestAnchor).await {
            Ok(ExactRpcResponse::LatestAnchor(anchor)) => {
                EvmTransportOutcome::Returned(EvmBlockResponse { anchor })
            }
            Ok(_) => indeterminate(EvmSafeFailure::ResponseInvalid {
                response_kind: EvmResponseInvalidKind::InvalidResult,
                size_class: EvmCoarseSizeClass::UpTo16Kib,
            }),
            Err(failure) => failure.into_outcome(),
        }
    }

    /// Performs one exact EIP-1898 `eth_getBalance` operation.
    pub async fn native_balance(
        &self,
        request: &EvmNativeBalanceRequest,
    ) -> EvmTransportOutcome<EvmQuantityResponse> {
        let route = match self.resolve_anchored_source(request.source()) {
            Ok(route) => route,
            Err(failure) => return failure.into_outcome(),
        };
        let account = match parse_canonical_address(request.account()) {
            Some(account) => account,
            None => return did_not_enter(EvmSafeFailure::RequestInvalid),
        };
        let block_hash = match parse_canonical_hash(request.source().anchor().hash()) {
            Some(block_hash) => block_hash,
            None => return did_not_enter(EvmSafeFailure::RequestInvalid),
        };
        match self
            .exchange(
                route,
                ExactRpcRequest::NativeBalance {
                    account,
                    block_hash,
                },
            )
            .await
        {
            Ok(ExactRpcResponse::NativeBalance(quantity)) => {
                EvmTransportOutcome::Returned(EvmQuantityResponse::new(quantity))
            }
            Ok(_) => indeterminate(EvmSafeFailure::ResponseInvalid {
                response_kind: EvmResponseInvalidKind::InvalidResult,
                size_class: EvmCoarseSizeClass::UpTo16Kib,
            }),
            Err(failure) => failure.into_outcome(),
        }
    }

    /// Performs one exact ERC-20 `decimals()` `eth_call`.
    pub async fn token_decimals(
        &self,
        request: &EvmTokenDecimalsRequest,
    ) -> EvmTransportOutcome<EvmTokenDecimalsResponse> {
        let route = match self.resolve_anchored_source(request.source()) {
            Ok(route) => route,
            Err(failure) => return failure.into_outcome(),
        };
        let contract = match parse_canonical_address(request.contract_address())
            .filter(|address| !address.is_zero())
        {
            Some(contract) => contract,
            None => return did_not_enter(EvmSafeFailure::RequestInvalid),
        };
        let block_hash = match parse_canonical_hash(request.source().anchor().hash()) {
            Some(block_hash) => block_hash,
            None => return did_not_enter(EvmSafeFailure::RequestInvalid),
        };
        match self
            .exchange(
                route,
                ExactRpcRequest::TokenDecimals {
                    contract,
                    block_hash,
                },
            )
            .await
        {
            Ok(ExactRpcResponse::TokenDecimals(decimals)) => {
                EvmTransportOutcome::Returned(EvmTokenDecimalsResponse { decimals })
            }
            Ok(_) => indeterminate(EvmSafeFailure::ResponseInvalid {
                response_kind: EvmResponseInvalidKind::InvalidResult,
                size_class: EvmCoarseSizeClass::UpTo16Kib,
            }),
            Err(failure) => failure.into_outcome(),
        }
    }

    /// Performs one exact ERC-20 `balanceOf(address)` `eth_call`.
    pub async fn token_balance(
        &self,
        request: &EvmTokenBalanceRequest,
    ) -> EvmTransportOutcome<EvmQuantityResponse> {
        let route = match self.resolve_anchored_source(request.source()) {
            Ok(route) => route,
            Err(failure) => return failure.into_outcome(),
        };
        let account = match parse_canonical_address(request.account()) {
            Some(account) => account,
            None => return did_not_enter(EvmSafeFailure::RequestInvalid),
        };
        let contract = match parse_canonical_address(request.contract_address())
            .filter(|address| !address.is_zero())
        {
            Some(contract) => contract,
            None => return did_not_enter(EvmSafeFailure::RequestInvalid),
        };
        let block_hash = match parse_canonical_hash(request.source().anchor().hash()) {
            Some(block_hash) => block_hash,
            None => return did_not_enter(EvmSafeFailure::RequestInvalid),
        };
        match self
            .exchange(
                route,
                ExactRpcRequest::TokenBalance {
                    contract,
                    account,
                    block_hash,
                },
            )
            .await
        {
            Ok(ExactRpcResponse::TokenBalance(quantity)) => {
                EvmTransportOutcome::Returned(EvmQuantityResponse::new(quantity))
            }
            Ok(_) => indeterminate(EvmSafeFailure::ResponseInvalid {
                response_kind: EvmResponseInvalidKind::InvalidResult,
                size_class: EvmCoarseSizeClass::UpTo16Kib,
            }),
            Err(failure) => failure.into_outcome(),
        }
    }

    /// Performs one exact number-to-hash `eth_getBlockByNumber` confirmation.
    pub async fn confirm_anchor(
        &self,
        request: &EvmAnchorConfirmationRequest,
    ) -> EvmTransportOutcome<EvmBlockResponse> {
        let source = match request.source() {
            Some(source) => source,
            None => return did_not_enter(EvmSafeFailure::RequestInvalid),
        };
        let route = match self.resolve_anchored_source(source) {
            Ok(route) => route,
            Err(failure) => return failure.into_outcome(),
        };
        let number = match source.anchor().number_quantity() {
            Ok(number) => number,
            Err(_) => return did_not_enter(EvmSafeFailure::RequestInvalid),
        };
        match self
            .exchange(route, ExactRpcRequest::ConfirmAnchor { number })
            .await
        {
            Ok(ExactRpcResponse::ConfirmAnchor(anchor)) => {
                EvmTransportOutcome::Returned(EvmBlockResponse { anchor })
            }
            Ok(_) => indeterminate(EvmSafeFailure::ResponseInvalid {
                response_kind: EvmResponseInvalidKind::InvalidResult,
                size_class: EvmCoarseSizeClass::UpTo16Kib,
            }),
            Err(failure) => failure.into_outcome(),
        }
    }

    fn resolve_binding(
        &self,
        binding: &mfm_evm::EvmNetworkBinding,
    ) -> std::result::Result<Arc<ResolvedGeneration>, BoundaryFailure> {
        let route = self
            .routes
            .resolve(binding.routing_generation_ref())
            .ok_or(BoundaryFailure::DidNotEnter(
                EvmSafeFailure::RoutingGenerationUnavailable,
            ))?;
        if route.descriptor.network_id != binding.network_id()
            || route.descriptor.chain_id != binding.chain_id()
        {
            return Err(BoundaryFailure::DidNotEnter(EvmSafeFailure::RequestInvalid));
        }
        Ok(route)
    }

    fn resolve_source(
        &self,
        source: &EvmCheckedSource,
    ) -> std::result::Result<Arc<ResolvedGeneration>, BoundaryFailure> {
        let route = self.resolve_binding(source.binding())?;
        if route.descriptor.source_ref != source.source_scope()
            || route.implementation_id != source.implementation_id()
        {
            return Err(BoundaryFailure::DidNotEnter(EvmSafeFailure::RequestInvalid));
        }
        Ok(route)
    }

    fn resolve_anchored_source(
        &self,
        source: &EvmAnchoredSource,
    ) -> std::result::Result<Arc<ResolvedGeneration>, BoundaryFailure> {
        source
            .anchor()
            .validate()
            .map_err(|_| BoundaryFailure::DidNotEnter(EvmSafeFailure::RequestInvalid))?;
        self.resolve_source(source.source())
    }

    async fn exchange(
        &self,
        route: Arc<ResolvedGeneration>,
        request: ExactRpcRequest,
    ) -> std::result::Result<ExactRpcResponse, BoundaryFailure> {
        let (_route_permit, _global_permit) = self.acquire_exchange_permits(&route).await?;
        let EncodedRpcRequest { operation, body } = request
            .encode()
            .map_err(|()| BoundaryFailure::DidNotEnter(EvmSafeFailure::RequestInvalid))?;
        let mut request = self
            .shared
            .client
            .post(route.endpoint.url.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(request_body(body));
        if let Some(authorization) = &route.authorization {
            request = request.header(AUTHORIZATION, authorization.value.clone());
        }
        debug!(operation = operation.method(), "evm audited rpc request");
        let mut response = request
            .send()
            .await
            .map_err(|_| BoundaryFailure::Indeterminate(EvmSafeFailure::TransportFailed))?;
        let status = response.status().as_u16();
        if response.status() != reqwest::StatusCode::OK {
            return Err(BoundaryFailure::Indeterminate(EvmSafeFailure::HttpStatus {
                status,
            }));
        }
        if let Some(length) = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|length| *length > MAX_RESPONSE_BYTES)
        {
            return Err(response_too_large(length));
        }
        let mut bytes = ZeroizingResponseBuffer::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| BoundaryFailure::Indeterminate(EvmSafeFailure::TransportFailed))?
        {
            bytes.extend(&chunk).map_err(response_too_large)?;
        }
        decode_response(operation, bytes.as_slice())
            .map_err(|failure| decode_boundary_failure(failure, bytes.len()))
    }

    async fn acquire_exchange_permits(
        &self,
        route: &Arc<ResolvedGeneration>,
    ) -> std::result::Result<(OwnedSemaphorePermit, OwnedSemaphorePermit), BoundaryFailure> {
        let route_permit = Arc::clone(&route.limit)
            .acquire_owned()
            .await
            .map_err(|_| BoundaryFailure::DidNotEnter(EvmSafeFailure::AccessCancelled))?;
        let global_permit = Arc::clone(&self.shared.global_limit)
            .acquire_owned()
            .await
            .map_err(|_| BoundaryFailure::DidNotEnter(EvmSafeFailure::AccessCancelled))?;
        Ok((route_permit, global_permit))
    }

    fn resolve_wallet_route(
        &self,
        route_generation_ref: &ContentRef,
        chain_id: u64,
    ) -> Result<Arc<ResolvedGeneration>, WalletRpcFailure> {
        let generation = EvmRoutingGenerationRef::from_content_ref(route_generation_ref.clone())
            .map_err(|_| WalletRpcFailure::GenerationFenced)?;
        let route = self
            .routes
            .resolve(&generation)
            .ok_or(WalletRpcFailure::GenerationFenced)?;
        if route.descriptor.chain_id != chain_id {
            return Err(WalletRpcFailure::GenerationFenced);
        }
        Ok(route)
    }

    async fn wallet_exchange(
        &self,
        route_generation_ref: &ContentRef,
        chain_id: u64,
        request: ExactRpcRequest,
    ) -> Result<ExactRpcResponse, WalletRpcFailure> {
        let route = self.resolve_wallet_route(route_generation_ref, chain_id)?;
        self.exchange(route, request)
            .await
            .map_err(wallet_boundary_failure)
    }

    pub(crate) async fn send_raw_transaction(
        &self,
        route_generation_ref: &ContentRef,
        chain_id: u64,
        signed: TransientSignedEip1559Envelope,
    ) -> Result<WalletBroadcastResponse, WalletRpcFailure> {
        match self
            .wallet_exchange(
                route_generation_ref,
                chain_id,
                ExactRpcRequest::SendRawTransaction { signed },
            )
            .await?
        {
            ExactRpcResponse::SendRawTransaction(response) => Ok(response),
            _ => Err(WalletRpcFailure::InvalidResponse),
        }
    }

    pub(crate) async fn transaction_by_hash(
        &self,
        route_generation_ref: &ContentRef,
        chain_id: u64,
        transaction_hash: B256,
    ) -> Result<Option<EvmWalletObservedTransaction>, WalletRpcFailure> {
        match self
            .wallet_exchange(
                route_generation_ref,
                chain_id,
                ExactRpcRequest::TransactionByHash { transaction_hash },
            )
            .await?
        {
            ExactRpcResponse::TransactionByHash(transaction) => Ok(transaction),
            _ => Err(WalletRpcFailure::InvalidResponse),
        }
    }

    pub(crate) async fn receipt_by_hash(
        &self,
        route_generation_ref: &ContentRef,
        chain_id: u64,
        transaction_hash: B256,
    ) -> Result<Option<EvmWalletReceipt>, WalletRpcFailure> {
        match self
            .wallet_exchange(
                route_generation_ref,
                chain_id,
                ExactRpcRequest::ReceiptByHash { transaction_hash },
            )
            .await?
        {
            ExactRpcResponse::ReceiptByHash(receipt) => Ok(receipt),
            _ => Err(WalletRpcFailure::InvalidResponse),
        }
    }

    pub(crate) async fn finalized_head(
        &self,
        route_generation_ref: &ContentRef,
        chain_id: u64,
    ) -> Result<EvmBlockAnchor, WalletRpcFailure> {
        match self
            .wallet_exchange(
                route_generation_ref,
                chain_id,
                ExactRpcRequest::FinalizedHead,
            )
            .await?
        {
            ExactRpcResponse::FinalizedHead(block) => Ok(block),
            _ => Err(WalletRpcFailure::InvalidResponse),
        }
    }

    pub(crate) async fn inclusion_block(
        &self,
        route_generation_ref: &ContentRef,
        chain_id: u64,
        number: U256,
    ) -> Result<Option<EvmBlockAnchor>, WalletRpcFailure> {
        match self
            .wallet_exchange(
                route_generation_ref,
                chain_id,
                ExactRpcRequest::InclusionBlock { number },
            )
            .await?
        {
            ExactRpcResponse::InclusionBlock(block) => Ok(block),
            _ => Err(WalletRpcFailure::InvalidResponse),
        }
    }
}

impl fmt::Debug for EvmJsonRpcTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvmJsonRpcTransport")
            .field("routes", &self.routes)
            .field("http", &"<shared-redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalletRpcFailure {
    GenerationFenced,
    AccessCancelled,
    UnavailableBeforeEntry,
    ResponseLost,
    DestinationRejected,
    InvalidResponse,
}

fn wallet_boundary_failure(failure: BoundaryFailure) -> WalletRpcFailure {
    match failure {
        BoundaryFailure::DidNotEnter(EvmSafeFailure::RoutingGenerationUnavailable) => {
            WalletRpcFailure::GenerationFenced
        }
        BoundaryFailure::DidNotEnter(EvmSafeFailure::AccessCancelled) => {
            WalletRpcFailure::AccessCancelled
        }
        BoundaryFailure::DidNotEnter(_) => WalletRpcFailure::UnavailableBeforeEntry,
        BoundaryFailure::Indeterminate(EvmSafeFailure::TransportFailed) => {
            WalletRpcFailure::ResponseLost
        }
        BoundaryFailure::Indeterminate(
            EvmSafeFailure::HttpStatus { .. } | EvmSafeFailure::JsonRpcError { .. },
        ) => WalletRpcFailure::DestinationRejected,
        BoundaryFailure::Indeterminate(_) => WalletRpcFailure::InvalidResponse,
    }
}

fn decode_boundary_failure(failure: DecodeFailure, response_len: usize) -> BoundaryFailure {
    match failure {
        DecodeFailure::MalformedEnvelope => {
            BoundaryFailure::Indeterminate(EvmSafeFailure::ResponseInvalid {
                response_kind: EvmResponseInvalidKind::MalformedEnvelope,
                size_class: EvmCoarseSizeClass::from_byte_length(response_len),
            })
        }
        DecodeFailure::MissingResult => {
            BoundaryFailure::Indeterminate(EvmSafeFailure::ResponseMissingResult {
                size_class: EvmCoarseSizeClass::from_byte_length(response_len),
            })
        }
        DecodeFailure::InvalidResult => {
            BoundaryFailure::Indeterminate(EvmSafeFailure::ResponseInvalid {
                response_kind: EvmResponseInvalidKind::InvalidResult,
                size_class: EvmCoarseSizeClass::from_byte_length(response_len),
            })
        }
        DecodeFailure::ResultTooLarge(result_len) => response_too_large(result_len),
        DecodeFailure::JsonRpcError(json_rpc_code) => {
            BoundaryFailure::Indeterminate(EvmSafeFailure::JsonRpcError { json_rpc_code })
        }
    }
}

enum BoundaryFailure {
    DidNotEnter(EvmSafeFailure),
    Indeterminate(EvmSafeFailure),
}

impl BoundaryFailure {
    fn into_outcome<T>(self) -> EvmTransportOutcome<T> {
        match self {
            Self::DidNotEnter(failure) => EvmTransportOutcome::DidNotEnter(failure),
            Self::Indeterminate(failure) => EvmTransportOutcome::Indeterminate(failure),
        }
    }
}

fn did_not_enter<T>(failure: EvmSafeFailure) -> EvmTransportOutcome<T> {
    EvmTransportOutcome::DidNotEnter(failure)
}

fn indeterminate<T>(failure: EvmSafeFailure) -> EvmTransportOutcome<T> {
    EvmTransportOutcome::Indeterminate(failure)
}

fn response_too_large(bytes: usize) -> BoundaryFailure {
    BoundaryFailure::Indeterminate(EvmSafeFailure::ResponseTooLarge {
        size_class: EvmCoarseSizeClass::from_byte_length(bytes),
    })
}

fn parse_canonical_address(raw: &str) -> Option<Address> {
    if raw.len() != 42
        || !raw.starts_with("0x")
        || !raw[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    raw.parse().ok()
}

fn parse_canonical_hash(raw: &str) -> Option<B256> {
    if raw.len() != 66
        || !raw.starts_with("0x")
        || !raw[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    raw.parse().ok()
}

fn canonical_json<T: Serialize>(value: &T) -> TransportResult<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value).map_err(|_| EvmTransportError::InvalidConfiguration)?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| EvmTransportError::InvalidConfiguration)
}

/// Returns the schema identity for one exact routing-generation descriptor.
pub fn routing_generation_descriptor_schema_id() -> TransportResult<SchemaId> {
    descriptor_schema_id("mfm.evm.routing-generation-descriptor")
}

/// Builds retained metadata for one routing-generation support object.
pub fn routing_generation_descriptor_support_contract(
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> TransportResult<RetainedValueContract> {
    descriptor_support_contract(
        routing_generation_descriptor_schema_id()?,
        "routing-generation-descriptor",
        role,
        evidence_contract_ref,
    )
}

/// Returns the schema identity for the aggregate routing-catalog descriptor.
pub fn routing_catalog_descriptor_schema_id() -> TransportResult<SchemaId> {
    descriptor_schema_id("mfm.evm.routing-catalog-descriptor")
}

/// Builds retained metadata for the aggregate routing-catalog support object.
pub fn routing_catalog_descriptor_support_contract(
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> TransportResult<RetainedValueContract> {
    descriptor_support_contract(
        routing_catalog_descriptor_schema_id()?,
        "routing-catalog-descriptor",
        role,
        evidence_contract_ref,
    )
}

fn descriptor_schema_id(name: &'static str) -> TransportResult<SchemaId> {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{name}:1").as_bytes()),
    )
    .map_err(|_| EvmTransportError::InvalidConfiguration)
}

fn descriptor_support_contract(
    schema_id: SchemaId,
    semantic_name: &'static str,
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> TransportResult<RetainedValueContract> {
    let semantic_type_id = SemanticTypeId::new(
        "mfm.evm",
        semantic_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("semantic:mfm.evm:{semantic_name}:1").as_bytes()),
    )
    .map_err(|_| EvmTransportError::InvalidConfiguration)?;
    RetainedValueContract::new(
        schema_id,
        semantic_type_id,
        role,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|_| EvmTransportError::InvalidConfiguration)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
