//! Affine qualification of the exact private EVM RPC inventory.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use mfm_canonical::sha256_digest_bytes;
use mfm_evm::EvmRoutingGenerationRef;
use mfm_ids::ContentRef;
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::{
    request_body, shared_http_runtime, EvmJsonRpcTransport, EvmRoutingCatalog, EvmTransportError,
    SharedHttpRuntime, TransportResult,
};

const INVENTORY_METHOD: &str = "mfm_qualifyRpcInventory";
const INVENTORY_PROTOCOL: &str = "mfm.evm.rpc-inventory-qualification.v1";
const MAX_INVENTORY_ROUTES: usize = 64;
const MAX_INVENTORY_PROOF_BYTES: usize = 16 * 1024;
const MAX_INVENTORY_RESPONSE_BYTES: usize = 48 * 1024;
const MAX_FINISH_AUTHORIZATION_BYTES: usize = 1024;

macro_rules! opaque_inventory_value {
    ($name:ident, $docs:literal) => {
        #[doc = $docs]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            /// Creates one opaque provider value from its fixed-width representation.
            pub const fn new(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// Returns the fixed-width opaque representation.
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(<opaque>)"))
            }
        }
    };
}

opaque_inventory_value!(
    EvmRpcTargetIdentity,
    "Opaque provider-selected identity of one exact RPC target."
);
opaque_inventory_value!(
    EvmRpcAssemblyLease,
    "Opaque identity of the assembly lease authorizing qualification."
);
opaque_inventory_value!(
    EvmRpcInventoryCheckpoint,
    "Opaque identity of the provider checkpoint authorizing qualification."
);
opaque_inventory_value!(
    EvmRpcRouteChallenge,
    "Opaque fresh challenge for one exact routing generation."
);

/// Provider-issued challenge for one exact routing generation and target.
#[derive(Clone, PartialEq, Eq)]
pub struct EvmRpcInventoryChallenge {
    route_generation_ref: ContentRef,
    target_identity: EvmRpcTargetIdentity,
    assembly_lease: EvmRpcAssemblyLease,
    checkpoint: EvmRpcInventoryCheckpoint,
    route_challenge: EvmRpcRouteChallenge,
    finish_authorization_commitment: [u8; 32],
}

impl EvmRpcInventoryChallenge {
    /// Constructs one challenge whose finish commitment is SHA-256 of the later opaque authorization.
    pub fn new(
        route_generation_ref: ContentRef,
        target_identity: EvmRpcTargetIdentity,
        assembly_lease: EvmRpcAssemblyLease,
        checkpoint: EvmRpcInventoryCheckpoint,
        route_challenge: EvmRpcRouteChallenge,
        finish_authorization_commitment: [u8; 32],
    ) -> Self {
        Self {
            route_generation_ref,
            target_identity,
            assembly_lease,
            checkpoint,
            route_challenge,
            finish_authorization_commitment,
        }
    }

    /// Returns the challenged immutable routing generation.
    pub const fn route_generation_ref(&self) -> &ContentRef {
        &self.route_generation_ref
    }

    /// Returns the opaque target identity.
    pub const fn target_identity(&self) -> EvmRpcTargetIdentity {
        self.target_identity
    }

    /// Returns the assembly lease identity.
    pub const fn assembly_lease(&self) -> EvmRpcAssemblyLease {
        self.assembly_lease
    }

    /// Returns the provider checkpoint identity.
    pub const fn checkpoint(&self) -> EvmRpcInventoryCheckpoint {
        self.checkpoint
    }

    /// Returns the exact per-route challenge.
    pub const fn route_challenge(&self) -> EvmRpcRouteChallenge {
        self.route_challenge
    }

    /// Returns the committed finish-authorization digest.
    pub const fn finish_authorization_commitment(&self) -> &[u8; 32] {
        &self.finish_authorization_commitment
    }
}

impl fmt::Debug for EvmRpcInventoryChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvmRpcInventoryChallenge")
            .field("route_generation_ref", &self.route_generation_ref)
            .field("target_identity", &self.target_identity)
            .field("assembly_lease", &self.assembly_lease)
            .field("checkpoint", &self.checkpoint)
            .field("route_challenge", &self.route_challenge)
            .field("finish_authorization_commitment", &"<opaque>")
            .finish()
    }
}

/// Ordered provider-issued challenge closure for the complete private inventory.
pub struct EvmRpcInventoryChallenges {
    challenges: Vec<EvmRpcInventoryChallenge>,
}

impl EvmRpcInventoryChallenges {
    /// Admits a bounded non-empty challenge closure.
    pub fn new(challenges: Vec<EvmRpcInventoryChallenge>) -> TransportResult<Self> {
        if challenges.is_empty() || challenges.len() > MAX_INVENTORY_ROUTES {
            return Err(EvmTransportError::InvalidInventoryChallenge);
        }
        Ok(Self { challenges })
    }

    /// Returns the challenges in provider-issued route order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &EvmRpcInventoryChallenge> {
        self.challenges.iter()
    }
}

impl fmt::Debug for EvmRpcInventoryChallenges {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvmRpcInventoryChallenges")
            .field("challenge_count", &self.challenges.len())
            .finish()
    }
}

/// Opaque identity of one completed, ordered target-proof exchange.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EvmRpcInventoryExchangeRef([u8; 32]);

impl EvmRpcInventoryExchangeRef {
    /// Returns the fixed-width exchange commitment.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for EvmRpcInventoryExchangeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EvmRpcInventoryExchangeRef(<opaque>)")
    }
}

/// Target-held bounded proof returned for one exact route challenge.
pub struct EvmRpcRouteProof {
    ordinal: u16,
    challenge: EvmRpcInventoryChallenge,
    proof: Box<[u8]>,
}

impl EvmRpcRouteProof {
    /// Returns the strict inventory ordinal signed by the target.
    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }

    /// Returns all public provider binding material signed by the target.
    pub const fn challenge(&self) -> &EvmRpcInventoryChallenge {
        &self.challenge
    }

    /// Returns the bounded opaque target-held signature or proof.
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
}

impl fmt::Debug for EvmRpcRouteProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvmRpcRouteProof")
            .field("ordinal", &self.ordinal)
            .field("challenge", &self.challenge)
            .field("proof", &"<opaque>")
            .finish()
    }
}

/// Complete ordered proof closure returned by the retained private endpoints.
pub struct EvmRpcInventoryProofs {
    proofs: Vec<EvmRpcRouteProof>,
    exchange_ref: EvmRpcInventoryExchangeRef,
}

impl EvmRpcInventoryProofs {
    /// Returns route proofs in exact catalog order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &EvmRpcRouteProof> {
        self.proofs.iter()
    }

    /// Returns the commitment to the complete ordered exchange.
    pub const fn exchange_ref(&self) -> EvmRpcInventoryExchangeRef {
        self.exchange_ref
    }
}

impl fmt::Debug for EvmRpcInventoryProofs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvmRpcInventoryProofs")
            .field("proof_count", &self.proofs.len())
            .field("exchange_ref", &self.exchange_ref)
            .finish()
    }
}

/// Opaque provider finish authorization opening the commitment in every challenge.
pub struct EvmRpcInventoryFinishAuthorization {
    value: Zeroizing<Vec<u8>>,
}

impl EvmRpcInventoryFinishAuthorization {
    /// Admits a non-empty bounded opaque provider authorization.
    pub fn new(value: Zeroizing<Vec<u8>>) -> TransportResult<Self> {
        if value.is_empty() || value.len() > MAX_FINISH_AUTHORIZATION_BYTES {
            return Err(EvmTransportError::InvalidInventoryFinishAuthorization);
        }
        Ok(Self { value })
    }
}

impl fmt::Debug for EvmRpcInventoryFinishAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EvmRpcInventoryFinishAuthorization(<redacted>)")
    }
}

struct CompletedExchange {
    exchange_ref: EvmRpcInventoryExchangeRef,
    finish_authorization_commitment: [u8; 32],
}

/// Affine owner of the exact private RPC catalog during provider qualification.
pub struct PendingEvmRpcInventory {
    routes: EvmRoutingCatalog,
    shared: Arc<SharedHttpRuntime>,
    completed: Option<CompletedExchange>,
}

impl PendingEvmRpcInventory {
    /// Consumes the only production-path catalog before a transport exists.
    pub fn new(routes: EvmRoutingCatalog) -> TransportResult<Self> {
        if routes.is_empty() || routes.len() > MAX_INVENTORY_ROUTES {
            return Err(EvmTransportError::InvalidConfiguration);
        }
        Ok(Self {
            routes,
            shared: shared_http_runtime()?,
            completed: None,
        })
    }

    /// Returns the exact secret-free catalog retained for the affine exchange.
    pub const fn routing_catalog_descriptor(&self) -> &mfm_evm::EvmRoutingCatalogDescriptor {
        self.routes.descriptor()
    }

    /// Consumes this pending handle through one fixed exchange against every retained route.
    pub async fn exchange(
        mut self,
        challenges: EvmRpcInventoryChallenges,
    ) -> TransportResult<(Self, EvmRpcInventoryProofs)> {
        if self.completed.is_some() {
            return Err(EvmTransportError::InvalidInventoryChallenge);
        }
        self.validate_challenges(&challenges)?;
        let mut proofs = Vec::with_capacity(challenges.challenges.len());
        for (index, challenge) in challenges.challenges.into_iter().enumerate() {
            let generation =
                EvmRoutingGenerationRef::from_content_ref(challenge.route_generation_ref.clone())
                    .map_err(|_| EvmTransportError::InvalidInventoryChallenge)?;
            let route = self
                .routes
                .resolve(&generation)
                .ok_or(EvmTransportError::InvalidInventoryChallenge)?;
            proofs.push(exchange_route(&self.shared, route, index, challenge).await?);
        }
        let exchange_ref = exchange_ref(&proofs)?;
        let finish_authorization_commitment = proofs
            .first()
            .ok_or(EvmTransportError::InvalidInventoryProof)?
            .challenge
            .finish_authorization_commitment;
        self.completed = Some(CompletedExchange {
            exchange_ref,
            finish_authorization_commitment,
        });
        Ok((
            self,
            EvmRpcInventoryProofs {
                proofs,
                exchange_ref,
            },
        ))
    }

    /// Completes the affine exchange and constructs its retained transport after provider finish.
    pub fn finish(
        self,
        authorization: EvmRpcInventoryFinishAuthorization,
    ) -> TransportResult<CompletedEvmRpcInventoryExchange> {
        let completed = self
            .completed
            .ok_or(EvmTransportError::InvalidInventoryFinishAuthorization)?;
        if sha256_digest_bytes(&authorization.value).as_bytes()
            != &completed.finish_authorization_commitment
        {
            return Err(EvmTransportError::InvalidInventoryFinishAuthorization);
        }
        Ok(CompletedEvmRpcInventoryExchange {
            transport: EvmJsonRpcTransport::from_inventory(self.routes, self.shared),
            exchange_ref: completed.exchange_ref,
        })
    }

    fn validate_challenges(&self, challenges: &EvmRpcInventoryChallenges) -> TransportResult<()> {
        if challenges.challenges.len() != self.routes.len() {
            return Err(EvmTransportError::InvalidInventoryChallenge);
        }
        let first = challenges
            .challenges
            .first()
            .ok_or(EvmTransportError::InvalidInventoryChallenge)?;
        let mut route_challenges = BTreeSet::new();
        for ((generation, _), challenge) in self
            .routes
            .generation_descriptors()
            .zip(&challenges.challenges)
        {
            let generation_ref = generation
                .to_content_ref()
                .map_err(|_| EvmTransportError::InvalidInventoryChallenge)?;
            if challenge.route_generation_ref != generation_ref
                || challenge.assembly_lease != first.assembly_lease
                || challenge.checkpoint != first.checkpoint
                || challenge.finish_authorization_commitment
                    != first.finish_authorization_commitment
                || !route_challenges.insert(challenge.route_challenge.0)
            {
                return Err(EvmTransportError::InvalidInventoryChallenge);
            }
        }
        Ok(())
    }
}

impl fmt::Debug for PendingEvmRpcInventory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingEvmRpcInventory")
            .field("route_count", &self.routes.len())
            .field("exchange_completed", &self.completed.is_some())
            .finish()
    }
}

/// Completed affine endpoint exchange owning the post-finish transport.
///
/// This is provider-neutral proof material, not production deployment
/// authority: the later authenticated provider/app bracket must validate the
/// target proofs and challenge issuer. A directly constructed
/// [`EvmJsonRpcTransport`] cannot mint this completed-exchange handle.
///
/// ```compile_fail
/// # use mfm_evm_live::transport::{
/// #     CompletedEvmRpcInventoryExchange, EvmJsonRpcTransport,
/// #     EvmRpcInventoryExchangeRef,
/// # };
/// fn forge(
///     transport: EvmJsonRpcTransport,
///     exchange_ref: EvmRpcInventoryExchangeRef,
/// ) -> CompletedEvmRpcInventoryExchange {
///     CompletedEvmRpcInventoryExchange { transport, exchange_ref }
/// }
/// ```
///
/// ```compile_fail
/// # use mfm_evm_live::transport::CompletedEvmRpcInventoryExchange;
/// fn duplicate(
///     value: &CompletedEvmRpcInventoryExchange,
/// ) -> CompletedEvmRpcInventoryExchange {
///     value.clone()
/// }
/// ```
pub struct CompletedEvmRpcInventoryExchange {
    transport: EvmJsonRpcTransport,
    exchange_ref: EvmRpcInventoryExchangeRef,
}

impl CompletedEvmRpcInventoryExchange {
    /// Returns the exact completed exchange commitment.
    pub const fn exchange_ref(&self) -> EvmRpcInventoryExchangeRef {
        self.exchange_ref
    }

    /// Borrows the exact post-qualification transport.
    pub const fn transport(&self) -> &EvmJsonRpcTransport {
        &self.transport
    }

    /// Consumes the completed exchange into its reusable transport.
    pub fn into_transport(self) -> EvmJsonRpcTransport {
        self.transport
    }
}

impl fmt::Debug for CompletedEvmRpcInventoryExchange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompletedEvmRpcInventoryExchange")
            .field("exchange_ref", &self.exchange_ref)
            .field("transport", &"<completed-redacted>")
            .finish()
    }
}

#[derive(Serialize)]
struct QualificationRequest<'a> {
    jsonrpc: &'static str,
    id: u16,
    method: &'static str,
    params: QualificationRequestParams<'a>,
}

#[derive(Serialize)]
struct QualificationRequestParams<'a> {
    protocol: &'static str,
    ordinal: u16,
    route_generation_ref: &'a ContentRef,
    target_identity: String,
    assembly_lease: String,
    checkpoint: String,
    route_challenge: String,
    finish_authorization_commitment: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QualificationResponse {
    jsonrpc: String,
    id: u16,
    result: QualificationResponseResult,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QualificationResponseResult {
    protocol: String,
    ordinal: u16,
    route_generation_ref: ContentRef,
    target_identity: String,
    assembly_lease: String,
    checkpoint: String,
    route_challenge: String,
    finish_authorization_commitment: String,
    proof: String,
}

async fn exchange_route(
    shared: &SharedHttpRuntime,
    route: Arc<super::ResolvedGeneration>,
    index: usize,
    challenge: EvmRpcInventoryChallenge,
) -> TransportResult<EvmRpcRouteProof> {
    let ordinal = u16::try_from(index).map_err(|_| EvmTransportError::InvalidInventoryChallenge)?;
    let body = serde_json::to_vec(&QualificationRequest {
        jsonrpc: "2.0",
        id: ordinal,
        method: INVENTORY_METHOD,
        params: QualificationRequestParams {
            protocol: INVENTORY_PROTOCOL,
            ordinal,
            route_generation_ref: &challenge.route_generation_ref,
            target_identity: hex::encode(challenge.target_identity.0),
            assembly_lease: hex::encode(challenge.assembly_lease.0),
            checkpoint: hex::encode(challenge.checkpoint.0),
            route_challenge: hex::encode(challenge.route_challenge.0),
            finish_authorization_commitment: hex::encode(challenge.finish_authorization_commitment),
        },
    })
    .map_err(|_| EvmTransportError::InvalidInventoryChallenge)?;
    let mut request = shared
        .client
        .post(route.endpoint.url.clone())
        .header(CONTENT_TYPE, "application/json")
        .body(request_body(Zeroizing::new(body)));
    if let Some(authorization) = &route.authorization {
        request = request.header(AUTHORIZATION, authorization.value.clone());
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| EvmTransportError::InventoryExchangeFailed)?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(EvmTransportError::InventoryExchangeFailed);
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_INVENTORY_RESPONSE_BYTES)
    {
        return Err(EvmTransportError::InvalidInventoryProof);
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_INVENTORY_RESPONSE_BYTES));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| EvmTransportError::InventoryExchangeFailed)?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_INVENTORY_RESPONSE_BYTES {
            return Err(EvmTransportError::InvalidInventoryProof);
        }
        bytes.extend_from_slice(&chunk);
    }
    let response: QualificationResponse =
        serde_json::from_slice(&bytes).map_err(|_| EvmTransportError::InvalidInventoryProof)?;
    let expected = &challenge;
    if response.jsonrpc != "2.0"
        || response.id != ordinal
        || response.result.protocol != INVENTORY_PROTOCOL
        || response.result.ordinal != ordinal
        || response.result.route_generation_ref != expected.route_generation_ref
        || response.result.target_identity != hex::encode(expected.target_identity.0)
        || response.result.assembly_lease != hex::encode(expected.assembly_lease.0)
        || response.result.checkpoint != hex::encode(expected.checkpoint.0)
        || response.result.route_challenge != hex::encode(expected.route_challenge.0)
        || response.result.finish_authorization_commitment
            != hex::encode(expected.finish_authorization_commitment)
    {
        return Err(EvmTransportError::InvalidInventoryProof);
    }
    let proof = decode_canonical_hex(&response.result.proof, MAX_INVENTORY_PROOF_BYTES)?;
    Ok(EvmRpcRouteProof {
        ordinal,
        challenge,
        proof: proof.into_boxed_slice(),
    })
}

fn decode_canonical_hex(value: &str, max_bytes: usize) -> TransportResult<Vec<u8>> {
    if value.is_empty()
        || value.len() > max_bytes.saturating_mul(2)
        || !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(EvmTransportError::InvalidInventoryProof);
    }
    hex::decode(value).map_err(|_| EvmTransportError::InvalidInventoryProof)
}

fn exchange_ref(proofs: &[EvmRpcRouteProof]) -> TransportResult<EvmRpcInventoryExchangeRef> {
    let mut bytes = Vec::new();
    for proof in proofs {
        append_frame(&mut bytes, &proof.ordinal.to_be_bytes())?;
        let reference = serde_json::to_vec(&proof.challenge.route_generation_ref)
            .map_err(|_| EvmTransportError::InvalidInventoryProof)?;
        append_frame(&mut bytes, &reference)?;
        append_frame(&mut bytes, &proof.challenge.target_identity.0)?;
        append_frame(&mut bytes, &proof.challenge.assembly_lease.0)?;
        append_frame(&mut bytes, &proof.challenge.checkpoint.0)?;
        append_frame(&mut bytes, &proof.challenge.route_challenge.0)?;
        append_frame(&mut bytes, &proof.challenge.finish_authorization_commitment)?;
        append_frame(&mut bytes, &proof.proof)?;
    }
    Ok(EvmRpcInventoryExchangeRef(
        *sha256_digest_bytes(&bytes).as_bytes(),
    ))
}

fn append_frame(output: &mut Vec<u8>, value: &[u8]) -> TransportResult<()> {
    let length =
        u32::try_from(value.len()).map_err(|_| EvmTransportError::InvalidInventoryProof)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

impl Drop for EvmRpcInventoryFinishAuthorization {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}
