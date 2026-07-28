//! Bounded exact-generation EVM JSON-RPC transport.
//!
//! Construction and routing selection perform no provider IO. Every public
//! operation method performs zero or one HTTP exchange against the exact
//! immutable generation carried by its typed request. Redirects, retries,
//! failover, current-route aliases, batching, and arbitrary JSON-RPC calls are
//! absent.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{Address, B256, U256};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_evm::{
    EvmAnchorConfirmationRequest, EvmAnchoredSource, EvmBlockAnchor, EvmBlockResponse,
    EvmChainIdentityRequest, EvmChainIdentityResponse, EvmCheckedSource, EvmCoarseSizeClass,
    EvmLatestAnchorRequest, EvmNativeBalanceRequest, EvmQuantityResponse, EvmResponseInvalidKind,
    EvmRoutingGenerationRef, EvmSafeFailure, EvmTokenBalanceRequest, EvmTokenDecimalsRequest,
    EvmTokenDecimalsResponse, EVM_READ_MAX_RESPONSE_BYTES,
};
use mfm_ids::{ContentRef, DigestAlgorithm, LocalPublicId, SchemaId, SemanticTypeId, StableId};
use mfm_program::{boundary_content_ref, ObservationOutcome};
use mfm_values::RetainedValueContract;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_LENGTH};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;
use tracing::debug;
use zeroize::Zeroizing;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_GLOBAL_IN_FLIGHT_EXCHANGES: usize = 64;
const MAX_IN_FLIGHT_EXCHANGES_PER_GENERATION: usize = 16;
const MAX_IDLE_CONNECTIONS_PER_HOST: usize = 16;
const JSON_RPC_ID: u64 = 1;
const JSON_RPC_IMPLEMENTATION_ID: &str = "mfm.evm.json-rpc.v1";
const ERC20_DECIMALS_SELECTOR: &str = "0x313ce567";
const ERC20_BALANCE_OF_SELECTOR: &str = "70a08231";
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
    value: Zeroizing<String>,
}

impl EvmRpcAuthorization {
    /// Admits one non-empty valid HTTP authorization header.
    pub fn new(value: Zeroizing<String>) -> TransportResult<Self> {
        if value.is_empty() || HeaderValue::from_str(value.as_str()).is_err() {
            return Err(EvmTransportError::InvalidConfiguration);
        }
        Ok(Self { value })
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

    /// Performs one exact `eth_chainId` operation.
    pub async fn chain_identity(
        &self,
        request: &EvmChainIdentityRequest,
    ) -> ObservationOutcome<EvmChainIdentityResponse, EvmSafeFailure> {
        let route = match self.resolve_binding(request.binding()) {
            Ok(route) => route,
            Err(failure) => return failure.into_outcome(),
        };
        let payload = match self.rpc_call(route.clone(), "eth_chainId", json!([])).await {
            Ok(payload) => payload,
            Err(failure) => return failure.into_outcome(),
        };
        let chain_id = match payload
            .result
            .as_str()
            .and_then(|raw| parse_quantity(raw).ok())
            .and_then(|quantity| u64::try_from(quantity).ok())
            .filter(|chain_id| *chain_id != 0)
        {
            Some(chain_id) => chain_id,
            None => return payload.invalid_result(),
        };
        ObservationOutcome::Returned(EvmChainIdentityResponse {
            chain_id,
            source_scope: route.descriptor.source_ref.clone(),
            implementation_id: route.implementation_id.clone(),
        })
    }

    /// Performs one exact latest `eth_getBlockByNumber` operation.
    pub async fn latest_anchor(
        &self,
        request: &EvmLatestAnchorRequest,
    ) -> ObservationOutcome<EvmBlockResponse, EvmSafeFailure> {
        let route = match self.resolve_source(request.source()) {
            Ok(route) => route,
            Err(failure) => return failure.into_outcome(),
        };
        let payload = match self
            .rpc_call(route, "eth_getBlockByNumber", json!(["latest", false]))
            .await
        {
            Ok(payload) => payload,
            Err(failure) => return failure.into_outcome(),
        };
        match parse_block(&payload.result) {
            Ok(anchor) => ObservationOutcome::Returned(EvmBlockResponse { anchor }),
            Err(()) => payload.invalid_result(),
        }
    }

    /// Performs one exact EIP-1898 `eth_getBalance` operation.
    pub async fn native_balance(
        &self,
        request: &EvmNativeBalanceRequest,
    ) -> ObservationOutcome<EvmQuantityResponse, EvmSafeFailure> {
        let route = match self.resolve_anchored_source(request.source()) {
            Ok(route) => route,
            Err(failure) => return failure.into_outcome(),
        };
        let account = match parse_canonical_address(request.account()) {
            Some(account) => account,
            None => return did_not_enter(EvmSafeFailure::RequestInvalid),
        };
        let payload = match self
            .rpc_call(
                route,
                "eth_getBalance",
                json!([
                    format!("{account:#x}"),
                    exact_hash_selector(request.source().anchor())
                ]),
            )
            .await
        {
            Ok(payload) => payload,
            Err(failure) => return failure.into_outcome(),
        };
        match payload
            .result
            .as_str()
            .and_then(|raw| parse_quantity(raw).ok())
        {
            Some(quantity) => ObservationOutcome::Returned(EvmQuantityResponse::new(quantity)),
            None => payload.invalid_result(),
        }
    }

    /// Performs one exact ERC-20 `decimals()` `eth_call`.
    pub async fn token_decimals(
        &self,
        request: &EvmTokenDecimalsRequest,
    ) -> ObservationOutcome<EvmTokenDecimalsResponse, EvmSafeFailure> {
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
        let payload = match self
            .rpc_call(
                route,
                "eth_call",
                json!([
                    {
                        "to": format!("{contract:#x}"),
                        "data": ERC20_DECIMALS_SELECTOR,
                    },
                    exact_hash_selector(request.source().anchor())
                ]),
            )
            .await
        {
            Ok(payload) => payload,
            Err(failure) => return failure.into_outcome(),
        };
        match payload.result.as_str().and_then(parse_abi_u8) {
            Some(decimals) => ObservationOutcome::Returned(EvmTokenDecimalsResponse { decimals }),
            None => payload.invalid_result(),
        }
    }

    /// Performs one exact ERC-20 `balanceOf(address)` `eth_call`.
    pub async fn token_balance(
        &self,
        request: &EvmTokenBalanceRequest,
    ) -> ObservationOutcome<EvmQuantityResponse, EvmSafeFailure> {
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
        let data = format!(
            "0x{ERC20_BALANCE_OF_SELECTOR}{:0>64}",
            hex::encode(account.as_slice())
        );
        let payload = match self
            .rpc_call(
                route,
                "eth_call",
                json!([
                    {
                        "to": format!("{contract:#x}"),
                        "data": data,
                    },
                    exact_hash_selector(request.source().anchor())
                ]),
            )
            .await
        {
            Ok(payload) => payload,
            Err(failure) => return failure.into_outcome(),
        };
        match payload.result.as_str().and_then(parse_abi_u256) {
            Some(quantity) => ObservationOutcome::Returned(EvmQuantityResponse::new(quantity)),
            None => payload.invalid_result(),
        }
    }

    /// Performs one exact number-to-hash `eth_getBlockByNumber` confirmation.
    pub async fn confirm_anchor(
        &self,
        request: &EvmAnchorConfirmationRequest,
    ) -> ObservationOutcome<EvmBlockResponse, EvmSafeFailure> {
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
        let payload = match self
            .rpc_call(
                route,
                "eth_getBlockByNumber",
                json!([encode_quantity(number), false]),
            )
            .await
        {
            Ok(payload) => payload,
            Err(failure) => return failure.into_outcome(),
        };
        match parse_block(&payload.result) {
            Ok(anchor) => ObservationOutcome::Returned(EvmBlockResponse { anchor }),
            Err(()) => payload.invalid_result(),
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

    async fn rpc_call(
        &self,
        route: Arc<ResolvedGeneration>,
        method: &'static str,
        params: Value,
    ) -> std::result::Result<RpcPayload, BoundaryFailure> {
        let bytes = self.raw_rpc_call(route, method, params).await?;
        parse_rpc_payload(&bytes)
    }

    async fn raw_rpc_call(
        &self,
        route: Arc<ResolvedGeneration>,
        method: &'static str,
        params: Value,
    ) -> std::result::Result<Vec<u8>, BoundaryFailure> {
        let _route_permit = Arc::clone(&route.limit)
            .acquire_owned()
            .await
            .map_err(|_| BoundaryFailure::DidNotEnter(EvmSafeFailure::AccessCancelled))?;
        let _global_permit = Arc::clone(&self.shared.global_limit)
            .acquire_owned()
            .await
            .map_err(|_| BoundaryFailure::DidNotEnter(EvmSafeFailure::AccessCancelled))?;
        let mut request = self
            .shared
            .client
            .post(route.endpoint.url.clone())
            .json(&json!({
                "jsonrpc": "2.0",
                "id": JSON_RPC_ID,
                "method": method,
                "params": params,
            }));
        if let Some(authorization) = &route.authorization {
            let value = HeaderValue::from_str(authorization.value.as_str())
                .map_err(|_| BoundaryFailure::DidNotEnter(EvmSafeFailure::ConfigurationInvalid))?;
            request = request.header(AUTHORIZATION, value);
        }
        debug!(operation = method, "evm audited rpc request");
        let mut response = request
            .send()
            .await
            .map_err(|_| BoundaryFailure::Indeterminate(EvmSafeFailure::TransportFailed))?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
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
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| BoundaryFailure::Indeterminate(EvmSafeFailure::TransportFailed))?
        {
            let next_len = bytes
                .len()
                .checked_add(chunk.len())
                .ok_or_else(|| response_too_large(usize::MAX))?;
            if next_len > MAX_RESPONSE_BYTES {
                return Err(response_too_large(next_len));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

impl crate::wallet_rpc::EvmWalletRpcClient for EvmJsonRpcTransport {
    fn exchange<'a>(
        &'a self,
        route_generation_ref: &'a ContentRef,
        chain_id: u64,
        method: &'static str,
        params: Value,
    ) -> crate::wallet_rpc::EvmWalletRpcFuture<'a> {
        Box::pin(async move {
            let generation =
                EvmRoutingGenerationRef::from_content_ref(route_generation_ref.clone())
                    .map_err(|_| crate::wallet_rpc::EvmWalletRpcFailure::GenerationFenced)?;
            let route = self
                .routes
                .resolve(&generation)
                .ok_or(crate::wallet_rpc::EvmWalletRpcFailure::GenerationFenced)?;
            if route.descriptor.chain_id != chain_id {
                return Err(crate::wallet_rpc::EvmWalletRpcFailure::GenerationFenced);
            }
            let bytes = self
                .raw_rpc_call(route, method, params)
                .await
                .map_err(wallet_boundary_failure)?;
            parse_wallet_rpc_payload(&bytes)
        })
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

fn wallet_boundary_failure(failure: BoundaryFailure) -> crate::wallet_rpc::EvmWalletRpcFailure {
    match failure {
        BoundaryFailure::DidNotEnter(EvmSafeFailure::RoutingGenerationUnavailable) => {
            crate::wallet_rpc::EvmWalletRpcFailure::GenerationFenced
        }
        BoundaryFailure::DidNotEnter(EvmSafeFailure::AccessCancelled) => {
            crate::wallet_rpc::EvmWalletRpcFailure::AccessCancelled
        }
        BoundaryFailure::DidNotEnter(_) => {
            crate::wallet_rpc::EvmWalletRpcFailure::UnavailableBeforeEntry
        }
        BoundaryFailure::Indeterminate(EvmSafeFailure::TransportFailed) => {
            crate::wallet_rpc::EvmWalletRpcFailure::ResponseLost
        }
        BoundaryFailure::Indeterminate(
            EvmSafeFailure::HttpStatus { .. } | EvmSafeFailure::JsonRpcError { .. },
        ) => crate::wallet_rpc::EvmWalletRpcFailure::DestinationRejected,
        BoundaryFailure::Indeterminate(_) => {
            crate::wallet_rpc::EvmWalletRpcFailure::InvalidResponse
        }
    }
}

fn parse_wallet_rpc_payload(
    bytes: &[u8],
) -> Result<crate::wallet_rpc::EvmWalletRpcResponse, crate::wallet_rpc::EvmWalletRpcFailure> {
    let body: Value = serde_json::from_slice(bytes)
        .map_err(|_| crate::wallet_rpc::EvmWalletRpcFailure::InvalidResponse)?;
    let object = body
        .as_object()
        .ok_or(crate::wallet_rpc::EvmWalletRpcFailure::InvalidResponse)?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || object.get("id").and_then(Value::as_u64) != Some(JSON_RPC_ID)
    {
        return Err(crate::wallet_rpc::EvmWalletRpcFailure::InvalidResponse);
    }
    match (object.get("result"), object.get("error")) {
        (Some(result), None) if object.len() == 3 => Ok(
            crate::wallet_rpc::EvmWalletRpcResponse::Result(result.clone()),
        ),
        (None, Some(error)) if object.len() == 3 => {
            let error = error
                .as_object()
                .ok_or(crate::wallet_rpc::EvmWalletRpcFailure::InvalidResponse)?;
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .ok_or(crate::wallet_rpc::EvmWalletRpcFailure::InvalidResponse)?;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .ok_or(crate::wallet_rpc::EvmWalletRpcFailure::InvalidResponse)?
                .to_owned();
            Ok(crate::wallet_rpc::EvmWalletRpcResponse::Error(
                crate::wallet_rpc::EvmWalletRpcError::new(code, message, error.len() == 2),
            ))
        }
        _ => Err(crate::wallet_rpc::EvmWalletRpcFailure::InvalidResponse),
    }
}

struct RpcPayload {
    result: Value,
    size_class: EvmCoarseSizeClass,
}

impl RpcPayload {
    fn invalid_result<T>(self) -> ObservationOutcome<T, EvmSafeFailure> {
        indeterminate(EvmSafeFailure::ResponseInvalid {
            response_kind: EvmResponseInvalidKind::InvalidResult,
            size_class: self.size_class,
        })
    }
}

enum BoundaryFailure {
    DidNotEnter(EvmSafeFailure),
    Indeterminate(EvmSafeFailure),
}

impl BoundaryFailure {
    fn into_outcome<T>(self) -> ObservationOutcome<T, EvmSafeFailure> {
        match self {
            Self::DidNotEnter(failure) => ObservationOutcome::DidNotEnter(failure),
            Self::Indeterminate(failure) => ObservationOutcome::Indeterminate(failure),
        }
    }
}

fn did_not_enter<T>(failure: EvmSafeFailure) -> ObservationOutcome<T, EvmSafeFailure> {
    ObservationOutcome::DidNotEnter(failure)
}

fn indeterminate<T>(failure: EvmSafeFailure) -> ObservationOutcome<T, EvmSafeFailure> {
    ObservationOutcome::Indeterminate(failure)
}

fn response_too_large(bytes: usize) -> BoundaryFailure {
    BoundaryFailure::Indeterminate(EvmSafeFailure::ResponseTooLarge {
        size_class: EvmCoarseSizeClass::from_byte_length(bytes),
    })
}

fn parse_rpc_payload(bytes: &[u8]) -> std::result::Result<RpcPayload, BoundaryFailure> {
    let size_class = EvmCoarseSizeClass::from_byte_length(bytes.len());
    let mut body = serde_json::from_slice::<Value>(bytes).map_err(|_| {
        BoundaryFailure::Indeterminate(EvmSafeFailure::ResponseInvalid {
            response_kind: EvmResponseInvalidKind::MalformedEnvelope,
            size_class,
        })
    })?;
    let object = body.as_object_mut().ok_or({
        BoundaryFailure::Indeterminate(EvmSafeFailure::ResponseInvalid {
            response_kind: EvmResponseInvalidKind::MalformedEnvelope,
            size_class,
        })
    })?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || object.get("id").and_then(Value::as_u64) != Some(JSON_RPC_ID)
    {
        return Err(BoundaryFailure::Indeterminate(
            EvmSafeFailure::ResponseInvalid {
                response_kind: EvmResponseInvalidKind::MalformedEnvelope,
                size_class,
            },
        ));
    }
    match (object.remove("result"), object.get("error")) {
        (Some(result), None) => {
            let result_size = serde_json::to_vec(&result)
                .map_err(|_| {
                    BoundaryFailure::Indeterminate(EvmSafeFailure::ResponseInvalid {
                        response_kind: EvmResponseInvalidKind::InvalidResult,
                        size_class,
                    })
                })?
                .len();
            if result_size > EVM_READ_MAX_RESPONSE_BYTES {
                return Err(BoundaryFailure::Indeterminate(
                    EvmSafeFailure::ResponseTooLarge {
                        size_class: EvmCoarseSizeClass::from_byte_length(result_size),
                    },
                ));
            }
            Ok(RpcPayload { result, size_class })
        }
        (None, Some(error)) => {
            let json_rpc_code = error
                .as_object()
                .and_then(|error| error.get("code"))
                .and_then(Value::as_i64)
                .ok_or({
                    BoundaryFailure::Indeterminate(EvmSafeFailure::ResponseInvalid {
                        response_kind: EvmResponseInvalidKind::MalformedEnvelope,
                        size_class,
                    })
                })?;
            Err(BoundaryFailure::Indeterminate(
                EvmSafeFailure::JsonRpcError { json_rpc_code },
            ))
        }
        (None, None) => Err(BoundaryFailure::Indeterminate(
            EvmSafeFailure::ResponseMissingResult { size_class },
        )),
        (Some(_), Some(_)) => Err(BoundaryFailure::Indeterminate(
            EvmSafeFailure::ResponseInvalid {
                response_kind: EvmResponseInvalidKind::MalformedEnvelope,
                size_class,
            },
        )),
    }
}

fn parse_block(value: &Value) -> std::result::Result<EvmBlockAnchor, ()> {
    let object = value.as_object().ok_or(())?;
    let number = quantity_field(object, "number")?;
    let hash = hash_field(object, "hash")?;
    Ok(EvmBlockAnchor::new(number, hash))
}

fn exact_hash_selector(anchor: &EvmBlockAnchor) -> Value {
    json!({
        "blockHash": anchor.hash(),
        "requireCanonical": true,
    })
}

fn encode_quantity(value: U256) -> String {
    format!("0x{value:x}")
}

fn parse_quantity(raw: &str) -> std::result::Result<U256, ()> {
    let digits = raw.strip_prefix("0x").ok_or(())?;
    if digits.is_empty()
        || digits.len() > 64
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(());
    }
    let value = U256::from_str_radix(digits, 16).map_err(|_| ())?;
    if encode_quantity(value) != raw {
        return Err(());
    }
    Ok(value)
}

fn parse_canonical_address(raw: &str) -> Option<Address> {
    let address = Address::from_str(raw).ok()?;
    (format!("{address:#x}") == raw).then_some(address)
}

fn parse_hash(raw: &str) -> std::result::Result<B256, ()> {
    if raw.len() != 66
        || !raw.starts_with("0x")
        || !raw[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(());
    }
    let hash = B256::from_str(raw).map_err(|_| ())?;
    if format!("{hash:#x}") != raw {
        return Err(());
    }
    Ok(hash)
}

fn parse_abi_u8(raw: &str) -> Option<u8> {
    let bytes = parse_bounded_hex(raw, 32)?;
    if bytes.len() != 32 || bytes[..31].iter().any(|byte| *byte != 0) {
        return None;
    }
    Some(bytes[31])
}

fn parse_abi_u256(raw: &str) -> Option<U256> {
    let bytes = parse_bounded_hex(raw, 32)?;
    let bytes: [u8; 32] = bytes.try_into().ok()?;
    Some(U256::from_be_bytes(bytes))
}

fn parse_bounded_hex(raw: &str, maximum: usize) -> Option<Vec<u8>> {
    let digits = raw.strip_prefix("0x")?;
    if !digits.len().is_multiple_of(2)
        || digits.len() / 2 > maximum
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    hex::decode(digits).ok()
}

fn string_field<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> std::result::Result<&'a str, ()> {
    object.get(field).and_then(Value::as_str).ok_or(())
}

fn quantity_field(object: &Map<String, Value>, field: &str) -> std::result::Result<U256, ()> {
    parse_quantity(string_field(object, field)?)
}

fn hash_field(object: &Map<String, Value>, field: &str) -> std::result::Result<B256, ()> {
    parse_hash(string_field(object, field)?)
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
