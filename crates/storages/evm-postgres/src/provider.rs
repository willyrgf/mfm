//! Authenticated client for the external wallet authority provider.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mfm_canonical::limits::{
    MAX_PROVIDER_DEPLOYMENT_ROUTES, MAX_PROVIDER_FINISH_AUTHORIZATION_BYTES,
    MAX_PROVIDER_MESSAGE_BYTES, MAX_PROVIDER_PROOF_BYTES,
};
use mfm_evm::{
    canonical_wallet_reference, ActivateEvmCandidateRequest, ActiveWalletCandidate,
    CompleteEvmNonceRequest, CompletedWalletNonce, EvmRoutingCatalogDescriptor, EvmWalletReference,
    ReserveEvmNonceRequest, ReservedWalletNonce, WalletNonceDomainActivationAttestation,
    WalletNonceDomainActivationRecord, WalletNonceStoreIncarnation, WalletNonceStoreLineageHead,
    WalletNonceStoreSuccessor,
};
use mfm_ids::{ContentDigest, ContentRef, StableId};
use mfm_journal::structured::{
    HistoryObject, LexicalValueRef, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_values::MfmValue;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::net::UnixStream;
use tokio::time::timeout;
use zeroize::{Zeroize, Zeroizing};

use crate::error::{PostgresEvmWalletError, Result};
use crate::support::{canonical_json, decode_canonical, reference_text};

const PROTOCOL_VERSION: u16 = 3;
const MAX_MESSAGE_BYTES: usize = MAX_PROVIDER_MESSAGE_BYTES;
const PROVIDER_IO_TIMEOUT: Duration = Duration::from_secs(15);
const AUTHENTICATION_DOMAIN: &[u8] = b"mfm.wallet-authority-provider.authentication.v1\0";
const ASSERTION_DOMAIN: &[u8] = b"mfm.wallet-authority-provider.assertion.v1\0";
const MAX_DEPLOYMENT_ROUTES: usize = MAX_PROVIDER_DEPLOYMENT_ROUTES;
/// Raw proof budget chosen so the complete FinishDeploymentAssembly JSON
/// envelope (hex-encoded proofs plus metadata for every admitted route) stays
/// within [`MAX_MESSAGE_BYTES`]. Hex encoding doubles each proof; the remaining
/// budget reserves framing for route refs, ordinals, and message keys.
const MAX_DEPLOYMENT_PROOF_BYTES: usize = MAX_PROVIDER_PROOF_BYTES;
const MAX_FINISH_AUTHORIZATION_BYTES: usize = MAX_PROVIDER_FINISH_AUTHORIZATION_BYTES;

/// Qualified public trust anchor for one external wallet authority provider.
///
/// The public key is deployment configuration, not state-authored evidence.
/// Its matching private key remains in the external provider process.
#[derive(Clone, PartialEq, Eq)]
pub struct WalletAuthorityProviderTrust {
    provider_id: StableId,
    fence_lineage_ref: EvmWalletReference,
    chain_registry_lineage_ref: EvmWalletReference,
    current_chain_registry_head_ref: EvmWalletReference,
    public_key: [u8; 32],
}

impl WalletAuthorityProviderTrust {
    /// Constructs a checked deployment trust anchor from an Ed25519 public key.
    pub fn new(
        provider_id: StableId,
        fence_lineage_ref: EvmWalletReference,
        chain_registry_lineage_ref: EvmWalletReference,
        current_chain_registry_head_ref: EvmWalletReference,
        public_key_hex: &str,
    ) -> Result<Self> {
        fence_lineage_ref
            .to_content_ref()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        chain_registry_lineage_ref
            .to_content_ref()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        current_chain_registry_head_ref
            .to_content_ref()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        if !references_are_distinct(&[
            &fence_lineage_ref,
            &chain_registry_lineage_ref,
            &current_chain_registry_head_ref,
        ]) {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        let decoded =
            hex::decode(public_key_hex).map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let public_key: [u8; 32] = decoded
            .try_into()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        Ok(Self {
            provider_id,
            fence_lineage_ref,
            chain_registry_lineage_ref,
            current_chain_registry_head_ref,
            public_key,
        })
    }

    /// Returns the stable provider identity.
    pub const fn provider_id(&self) -> &StableId {
        &self.provider_id
    }

    /// Returns the admitted non-rollback provider fence lineage.
    pub const fn fence_lineage_ref(&self) -> &EvmWalletReference {
        &self.fence_lineage_ref
    }

    /// Returns the admitted append-only chain-registry lineage.
    pub const fn chain_registry_lineage_ref(&self) -> &EvmWalletReference {
        &self.chain_registry_lineage_ref
    }

    /// Returns the provider-enforced exact current chain-registry head.
    pub const fn current_chain_registry_head_ref(&self) -> &EvmWalletReference {
        &self.current_chain_registry_head_ref
    }
}

impl fmt::Debug for WalletAuthorityProviderTrust {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WalletAuthorityProviderTrust")
            .field("provider_id", &self.provider_id)
            .field("fence_lineage_ref", &self.fence_lineage_ref)
            .field(
                "chain_registry_lineage_ref",
                &self.chain_registry_lineage_ref,
            )
            .field(
                "current_chain_registry_head_ref",
                &self.current_chain_registry_head_ref,
            )
            .field("public_key", &"[public key]")
            .finish()
    }
}

/// Concrete authenticated client for the separate wallet authority provider.
///
/// Cloning this value clones only connection configuration and a public trust
/// anchor. Every authority-bearing operation still requires a fresh live,
/// authenticated provider channel and an affine server-tracked lease.
#[derive(Clone)]
pub struct WalletAuthorityProviderClient {
    endpoint: Arc<PathBuf>,
    trust: Arc<WalletAuthorityProviderTrust>,
}

impl WalletAuthorityProviderClient {
    /// Connects to and authenticates the configured separate provider process.
    pub async fn connect_unix(
        endpoint: impl AsRef<Path>,
        trust: WalletAuthorityProviderTrust,
    ) -> Result<Self> {
        let endpoint = endpoint.as_ref();
        if endpoint.as_os_str().is_empty() {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        let client = Self {
            endpoint: Arc::new(endpoint.to_path_buf()),
            trust: Arc::new(trust),
        };
        client.channel().await?;
        Ok(client)
    }

    /// Returns the admitted provider identity without exposing its endpoint.
    pub fn provider_id(&self) -> &StableId {
        self.trust.provider_id()
    }

    async fn channel(&self) -> Result<ProviderChannel> {
        let stream = timeout(PROVIDER_IO_TIMEOUT, UnixStream::connect(&*self.endpoint))
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        let (read, write) = tokio::io::split(stream);
        let mut channel = ProviderChannel {
            read: BufReader::new(read),
            write,
            provider_id: self.trust.provider_id.as_str().to_owned(),
            authentication_challenge: [0; 32],
            public_key: self.trust.public_key,
        };
        SystemRandom::new()
            .fill(&mut channel.authentication_challenge)
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        let request = ProviderRequest::Authenticate {
            version: PROTOCOL_VERSION,
            provider_id: self.trust.provider_id.as_str().to_owned(),
            fence_lineage_ref: self.trust.fence_lineage_ref.clone(),
            chain_registry_lineage_ref: self.trust.chain_registry_lineage_ref.clone(),
            current_chain_registry_head_ref: self.trust.current_chain_registry_head_ref.clone(),
            challenge: hex::encode(channel.authentication_challenge),
        };
        channel.send(&request).await?;
        let reply: ProviderReply = channel.receive().await?;
        let ProviderReply::Authenticated { signature } = reply else {
            return Err(PostgresEvmWalletError::FenceRejected);
        };
        let signature = Zeroizing::new(signature);
        let signature_bytes = decode_signature(&signature)?;
        let mut signed = Vec::with_capacity(
            AUTHENTICATION_DOMAIN.len()
                + channel.authentication_challenge.len()
                + self.trust.provider_id.as_str().len()
                + self.trust.fence_lineage_ref.content_digest().len()
                + self.trust.chain_registry_lineage_ref.content_digest().len()
                + self
                    .trust
                    .current_chain_registry_head_ref
                    .content_digest()
                    .len(),
        );
        signed.extend_from_slice(AUTHENTICATION_DOMAIN);
        signed.extend_from_slice(&channel.authentication_challenge);
        signed.extend_from_slice(self.trust.provider_id.as_str().as_bytes());
        signed.extend_from_slice(self.trust.fence_lineage_ref.content_digest().as_bytes());
        signed.extend_from_slice(
            self.trust
                .chain_registry_lineage_ref
                .content_digest()
                .as_bytes(),
        );
        signed.extend_from_slice(
            self.trust
                .current_chain_registry_head_ref
                .content_digest()
                .as_bytes(),
        );
        UnparsedPublicKey::new(&ED25519, self.trust.public_key)
            .verify(&signed, signature_bytes.as_ref())
            .map_err(|_| PostgresEvmWalletError::FenceRejected)?;
        Ok(channel)
    }

    /// Authenticates one complete routing catalog through the external issuer.
    pub async fn qualify_routing_catalog(
        &self,
        descriptor: &EvmRoutingCatalogDescriptor,
    ) -> Result<QualifiedEvmRoutingCatalog> {
        descriptor
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        if descriptor.chain_instances().iter().any(|attestation| {
            attestation.declaration().chain_registry_lineage_ref()
                != &self.trust.chain_registry_lineage_ref
        }) || routing_catalog_contains_authority_reference(
            descriptor,
            &self.trust.fence_lineage_ref,
        ) {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        if descriptor.chain_registry_head_ref() != &self.trust.current_chain_registry_head_ref {
            return Err(PostgresEvmWalletError::FenceRejected);
        }
        let request = ProviderRequest::QualifyRoutingCatalog {
            descriptor: descriptor.clone(),
            current_chain_registry_head_ref: self.trust.current_chain_registry_head_ref.clone(),
        };
        let mut channel = self.channel().await?;
        channel.send(&request).await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::RoutingCatalogQualified {
                chain_registry_head_ref,
                provider_fence_head_ref,
                target_attestation,
            } => {
                let target_attestation = Zeroizing::new(target_attestation);
                self.verify_assertion(
                    &channel,
                    "qualify-routing-catalog",
                    &(
                        descriptor,
                        &self.trust.current_chain_registry_head_ref,
                        &chain_registry_head_ref,
                        &provider_fence_head_ref,
                    ),
                    &target_attestation,
                )?;
                if &chain_registry_head_ref != descriptor.chain_registry_head_ref() {
                    return Err(PostgresEvmWalletError::InvalidAuthority);
                }
                provider_fence_head_ref
                    .to_content_ref()
                    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
                if routing_catalog_contains_authority_reference(
                    descriptor,
                    &provider_fence_head_ref,
                ) {
                    return Err(PostgresEvmWalletError::InvalidAuthority);
                }
                Ok(QualifiedEvmRoutingCatalog {
                    descriptor: descriptor.clone(),
                    provider_fence_head_ref,
                    client: self.clone(),
                })
            }
            ProviderReply::Unavailable => Err(PostgresEvmWalletError::Unavailable),
            ProviderReply::Rejected | ProviderReply::Integrity => {
                Err(PostgresEvmWalletError::FenceRejected)
            }
            _ => Err(PostgresEvmWalletError::InvalidAuthority),
        }
    }

    pub(crate) async fn qualify_domain(
        &self,
        attestation: &WalletNonceDomainActivationAttestation,
        store_incarnation: &WalletNonceStoreIncarnation,
        current_public_lineage_head: &HistoryObject,
    ) -> Result<OfflineActivationVerifier> {
        attestation
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        store_incarnation
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        current_public_lineage_head
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let request = ProviderRequest::QualifyDomain {
            attestation: attestation.clone(),
            store_incarnation: store_incarnation.clone(),
            current_public_lineage_head: current_public_lineage_head.clone(),
        };
        let mut channel = self.channel().await?;
        channel.send(&request).await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::DomainQualified {
                provider_fence_head_ref,
                target_attestation,
            } => {
                let target_attestation = Zeroizing::new(target_attestation);
                self.verify_assertion(
                    &channel,
                    "qualify-domain",
                    &(
                        attestation,
                        store_incarnation,
                        current_public_lineage_head,
                        &provider_fence_head_ref,
                    ),
                    &target_attestation,
                )?;
                provider_fence_head_ref
                    .to_content_ref()
                    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
                Ok(OfflineActivationVerifier {
                    exact_attestation: attestation.clone(),
                    store_incarnation: store_incarnation.clone(),
                    provider_fence_head_ref,
                    client: self.clone(),
                })
            }
            ProviderReply::Unavailable => Err(PostgresEvmWalletError::Unavailable),
            ProviderReply::Rejected | ProviderReply::Integrity => {
                Err(PostgresEvmWalletError::FenceRejected)
            }
            _ => Err(PostgresEvmWalletError::InvalidAuthority),
        }
    }

    pub(crate) async fn open_registry_maintenance(&self) -> Result<ProviderRegistryMaintenance> {
        let request = ProviderRequest::OpenRegistryMaintenance;
        let mut channel = self.channel().await?;
        channel.send(&request).await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::RegistryMaintenanceOpened {
                registry_lineage_ref,
                provider_fence_head_ref,
                provider_attestation,
            } => {
                let provider_attestation = Zeroizing::new(provider_attestation);
                self.verify_assertion(
                    &channel,
                    "open-registry-maintenance",
                    &(&registry_lineage_ref, &provider_fence_head_ref),
                    &provider_attestation,
                )?;
                registry_lineage_ref
                    .to_content_ref()
                    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
                provider_fence_head_ref
                    .to_content_ref()
                    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
                Ok(ProviderRegistryMaintenance {
                    client: self.clone(),
                    registry_lineage_ref,
                    provider_fence_head_ref,
                })
            }
            ProviderReply::Unavailable => Err(PostgresEvmWalletError::Unavailable),
            ProviderReply::Rejected | ProviderReply::Integrity => {
                Err(PostgresEvmWalletError::FenceRejected)
            }
            _ => Err(PostgresEvmWalletError::InvalidAuthority),
        }
    }

    fn verify_assertion<T: Serialize>(
        &self,
        channel: &ProviderChannel,
        assertion_kind: &str,
        payload: &T,
        signature_hex: &str,
    ) -> Result<()> {
        let digest = mfm_journal::structured::domain_content_digest(
            "mfm.wallet-authority-provider.assertion-payload.v1",
            payload,
        )
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let signature = decode_signature(signature_hex)?;
        let mut signed = Vec::with_capacity(
            ASSERTION_DOMAIN.len()
                + channel.authentication_challenge.len()
                + assertion_kind.len()
                + digest.as_str().len(),
        );
        signed.extend_from_slice(ASSERTION_DOMAIN);
        signed.extend_from_slice(&channel.authentication_challenge);
        signed.extend_from_slice(assertion_kind.as_bytes());
        signed.extend_from_slice(digest.as_str().as_bytes());
        let verified = UnparsedPublicKey::new(&ED25519, self.trust.public_key)
            .verify(&signed, signature.as_ref())
            .map_err(|_| PostgresEvmWalletError::FenceRejected);
        verified
    }

    fn verify_persisted_mutation(
        &self,
        proof: &str,
        expected_context: Option<&ProviderTargetContext>,
        expected_operation_key: &str,
        mutation: &ProviderMutation,
    ) -> Result<ProviderTargetContext> {
        verify_persisted_mutation_proof(
            &self.trust.public_key,
            self.trust.provider_id.as_str(),
            proof,
            expected_context,
            expected_operation_key,
            mutation,
        )
    }
}

impl fmt::Debug for WalletAuthorityProviderClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WalletAuthorityProviderClient")
            .field("provider_id", &self.trust.provider_id)
            .field("endpoint", &"[authenticated channel]")
            .finish()
    }
}

/// Opaque provider-qualified routing catalog admitted for one deployment.
///
/// The value is intentionally not serializable and cannot be constructed from
/// persisted catalog bytes without a fresh authenticated provider exchange.
pub struct QualifiedEvmRoutingCatalog {
    descriptor: EvmRoutingCatalogDescriptor,
    provider_fence_head_ref: EvmWalletReference,
    client: WalletAuthorityProviderClient,
}

impl QualifiedEvmRoutingCatalog {
    /// Returns the exact qualified secret-free catalog descriptor.
    pub const fn descriptor(&self) -> &EvmRoutingCatalogDescriptor {
        &self.descriptor
    }

    /// Returns the catalog content identity authenticated by the provider.
    pub fn content_ref(&self) -> Result<ContentRef> {
        self.descriptor
            .content_ref()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)
    }

    /// Builds the exact routing-policy history object admitted at run start.
    pub fn routing_policy_object(&self) -> Result<HistoryObject> {
        let object_type = StableId::new(ADMISSION_ROUTING_POLICY_OBJECT_TYPE)
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let schema_id = EvmRoutingCatalogDescriptor::schema_id()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let canonical = self
            .descriptor
            .canonical()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        HistoryObject::new(object_type, schema_id, canonical.as_str())
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)
    }

    /// Returns the provider fence head that qualified this catalog.
    pub const fn provider_fence_head_ref(&self) -> &EvmWalletReference {
        &self.provider_fence_head_ref
    }

    /// Consumes this qualification and opens one provider-tracked assembly lease.
    pub async fn begin_deployment_assembly(
        self,
        binding: DeploymentAssemblyBinding,
    ) -> Result<PendingDeploymentAssembly> {
        let mut channel = self.client.channel().await?;
        channel
            .send(&ProviderRequest::BeginDeploymentAssembly {
                descriptor: self.descriptor.clone(),
                provider_fence_head_ref: self.provider_fence_head_ref.clone(),
                binding: binding.clone(),
            })
            .await?;
        let reply = channel.receive::<ProviderReply>().await?;
        let ProviderReply::DeploymentAssemblyBegun {
            assembly_lease,
            checkpoint,
            finish_authorization_commitment,
            route_challenges,
            provider_attestation,
        } = reply
        else {
            return Err(reply_to_assembly_error(reply));
        };
        let provider_attestation = Zeroizing::new(provider_attestation);
        self.client.verify_assertion(
            &channel,
            "begin-deployment-assembly",
            &(
                &self.descriptor,
                &self.provider_fence_head_ref,
                &binding,
                &assembly_lease,
                &checkpoint,
                &finish_authorization_commitment,
                &route_challenges,
            ),
            &provider_attestation,
        )?;
        let lease = decode_fixed_opaque(&assembly_lease)?;
        let checkpoint = decode_fixed_opaque(&checkpoint)?;
        let finish_authorization_commitment =
            decode_fixed_opaque(&finish_authorization_commitment)?;
        validate_deployment_route_count(
            route_challenges.len(),
            self.descriptor.generations().len(),
        )?;
        let challenges = route_challenges
            .into_iter()
            .zip(self.descriptor.generations())
            .map(|(wire, generation)| {
                let route_generation_ref = generation
                    .generation_ref()
                    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?
                    .to_content_ref()
                    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
                if wire.route_generation_ref != route_generation_ref {
                    return Err(PostgresEvmWalletError::InvalidAuthority);
                }
                Ok(DeploymentAssemblyRouteChallenge {
                    route_generation_ref,
                    target_identity: decode_fixed_opaque(&wire.target_identity)?,
                    route_challenge: decode_fixed_opaque(&wire.route_challenge)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(PendingDeploymentAssembly {
            descriptor: self.descriptor,
            provider_fence_head_ref: self.provider_fence_head_ref,
            client: self.client,
            binding,
            assembly_lease: lease,
            checkpoint,
            finish_authorization_commitment,
            challenges,
        })
    }
}

/// Complete secret-free semantic and physical closure fixed by an assembly lease.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentAssemblyBinding {
    activation: WalletNonceDomainActivationAttestation,
    store_incarnation: WalletNonceStoreIncarnation,
    current_public_lineage_head: HistoryObject,
    wallet_provider_fence_head_ref: EvmWalletReference,
    semantic_signer_id: StableId,
    signer_generation_ref: ContentRef,
    signer_fence_ref: ContentRef,
    direct_sign_exclusion_ref: ContentRef,
    semantic_contract_refs: Vec<ContentRef>,
    release_history_digests: Vec<ContentDigest>,
}

impl DeploymentAssemblyBinding {
    /// Constructs the exact bounded deployment closure sent to Begin and Finish.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        activation: WalletNonceDomainActivationAttestation,
        store_incarnation: WalletNonceStoreIncarnation,
        current_public_lineage_head: HistoryObject,
        wallet_provider_fence_head_ref: EvmWalletReference,
        semantic_signer_id: StableId,
        signer_generation_ref: ContentRef,
        signer_fence_ref: ContentRef,
        direct_sign_exclusion_ref: ContentRef,
        semantic_contract_refs: Vec<ContentRef>,
        release_history_digests: Vec<ContentDigest>,
    ) -> Result<Self> {
        if activation.validate().is_err()
            || store_incarnation.validate().is_err()
            || current_public_lineage_head.validate().is_err()
            || wallet_provider_fence_head_ref.to_content_ref().is_err()
            || semantic_contract_refs.len() != 6
            || release_history_digests.len() != 6
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(Self {
            activation,
            store_incarnation,
            current_public_lineage_head,
            wallet_provider_fence_head_ref,
            semantic_signer_id,
            signer_generation_ref,
            signer_fence_ref,
            direct_sign_exclusion_ref,
            semantic_contract_refs,
            release_history_digests,
        })
    }
}

impl fmt::Debug for QualifiedEvmRoutingCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QualifiedEvmRoutingCatalog")
            .field("catalog_ref", &self.descriptor.content_ref().ok())
            .field("provider_fence_head_ref", &self.provider_fence_head_ref)
            .finish()
    }
}

/// One provider-issued challenge in exact catalog order.
pub struct DeploymentAssemblyRouteChallenge {
    route_generation_ref: ContentRef,
    target_identity: [u8; 32],
    route_challenge: [u8; 32],
}

impl DeploymentAssemblyRouteChallenge {
    /// Returns the exact challenged routing generation.
    pub const fn route_generation_ref(&self) -> &ContentRef {
        &self.route_generation_ref
    }

    /// Returns the opaque provider-selected target identity.
    pub const fn target_identity(&self) -> &[u8; 32] {
        &self.target_identity
    }

    /// Returns the fresh per-route target challenge.
    pub const fn route_challenge(&self) -> &[u8; 32] {
        &self.route_challenge
    }
}

/// Affine provider lease awaiting the exact private target-proof closure.
pub struct PendingDeploymentAssembly {
    descriptor: EvmRoutingCatalogDescriptor,
    provider_fence_head_ref: EvmWalletReference,
    client: WalletAuthorityProviderClient,
    binding: DeploymentAssemblyBinding,
    assembly_lease: [u8; 32],
    checkpoint: [u8; 32],
    finish_authorization_commitment: [u8; 32],
    challenges: Vec<DeploymentAssemblyRouteChallenge>,
}

impl PendingDeploymentAssembly {
    /// Returns challenges in exact provider-issued catalog order.
    pub fn route_challenges(
        &self,
    ) -> impl ExactSizeIterator<Item = &DeploymentAssemblyRouteChallenge> {
        self.challenges.iter()
    }

    /// Returns the opaque assembly lease used by every target challenge.
    pub const fn assembly_lease(&self) -> &[u8; 32] {
        &self.assembly_lease
    }

    /// Returns the provider checkpoint used by every target challenge.
    pub const fn checkpoint(&self) -> &[u8; 32] {
        &self.checkpoint
    }

    /// Returns the commitment to the finish authorization withheld by the provider.
    pub const fn finish_authorization_commitment(&self) -> &[u8; 32] {
        &self.finish_authorization_commitment
    }

    /// Consumes the lease and asks the provider to validate the complete ordered proof closure.
    pub async fn finish(
        self,
        exchange_ref: [u8; 32],
        proofs: Vec<DeploymentAssemblyRouteProof>,
    ) -> Result<FinishedDeploymentAssembly> {
        if proofs.len() != self.challenges.len()
            || proofs.iter().enumerate().any(|(index, proof)| {
                usize::from(proof.ordinal) != index
                    || proof.route_generation_ref != self.challenges[index].route_generation_ref
                    || proof.proof.is_empty()
                    || proof.proof.len() > MAX_DEPLOYMENT_PROOF_BYTES
            })
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        let wire_proofs = proofs
            .into_iter()
            .map(|proof| DeploymentAssemblyRouteProofWire {
                ordinal: proof.ordinal,
                route_generation_ref: proof.route_generation_ref,
                proof: hex::encode(proof.proof),
            })
            .collect::<Vec<_>>();
        let exchange_ref_hex = hex::encode(exchange_ref);
        let assembly_lease_hex = hex::encode(self.assembly_lease);
        let checkpoint_hex = hex::encode(self.checkpoint);
        // Account for the complete canonical record envelope so an accepted
        // proof collection can never exceed the store frame after encoding.
        let finish_request = ProviderRequest::FinishDeploymentAssembly {
            descriptor: self.descriptor.clone(),
            provider_fence_head_ref: self.provider_fence_head_ref.clone(),
            binding: self.binding.clone(),
            assembly_lease: assembly_lease_hex.clone(),
            checkpoint: checkpoint_hex.clone(),
            exchange_ref: exchange_ref_hex.clone(),
            proofs: wire_proofs.clone(),
        };
        let encoded_finish = Zeroizing::new(
            serde_json::to_vec(&finish_request)
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        );
        if encoded_finish.len() > MAX_MESSAGE_BYTES || encoded_finish.contains(&b'\n') {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        let mut channel = self.client.channel().await?;
        channel.send(&finish_request).await?;
        let reply = channel.receive::<ProviderReply>().await?;
        let ProviderReply::DeploymentAssemblyFinished {
            mut finish_authorization,
            provider_attestation,
        } = reply
        else {
            return Err(reply_to_assembly_error(reply));
        };
        let decoded_finish_authorization = hex::decode(finish_authorization.as_str())
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        finish_authorization.zeroize();
        let finish_authorization = Zeroizing::new(decoded_finish_authorization);
        validate_finish_authorization(
            &finish_authorization,
            &self.finish_authorization_commitment,
        )?;
        let provider_attestation = Zeroizing::new(provider_attestation);
        let finish_authorization_hex = Zeroizing::new(hex::encode(finish_authorization.as_slice()));
        self.client.verify_assertion(
            &channel,
            "finish-deployment-assembly",
            &(
                &self.descriptor,
                &self.provider_fence_head_ref,
                &self.binding,
                &assembly_lease_hex,
                &checkpoint_hex,
                &exchange_ref_hex,
                &wire_proofs,
                finish_authorization_hex.as_str(),
            ),
            &provider_attestation,
        )?;
        Ok(FinishedDeploymentAssembly {
            descriptor: self.descriptor,
            exchange_ref,
            finish_authorization,
        })
    }
}

/// One bounded target proof submitted to the authenticated provider.
pub struct DeploymentAssemblyRouteProof {
    ordinal: u16,
    route_generation_ref: ContentRef,
    proof: Box<[u8]>,
}

impl DeploymentAssemblyRouteProof {
    /// Constructs one exact ordered target proof.
    pub fn new(ordinal: u16, route_generation_ref: ContentRef, proof: Box<[u8]>) -> Result<Self> {
        if proof.is_empty() || proof.len() > MAX_DEPLOYMENT_PROOF_BYTES {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(Self {
            ordinal,
            route_generation_ref,
            proof,
        })
    }
}

fn validate_deployment_route_count(route_count: usize, expected_count: usize) -> Result<()> {
    if route_count == 0 || route_count > MAX_DEPLOYMENT_ROUTES || route_count != expected_count {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

/// Nonforgeable provider-finished evidence consumed by the application assembly bracket.
pub struct FinishedDeploymentAssembly {
    descriptor: EvmRoutingCatalogDescriptor,
    exchange_ref: [u8; 32],
    finish_authorization: Zeroizing<Vec<u8>>,
}

impl FinishedDeploymentAssembly {
    /// Consumes the evidence into the provider-neutral finish authorization and exact bindings.
    pub fn into_finish_material(
        self,
    ) -> (EvmRoutingCatalogDescriptor, [u8; 32], Zeroizing<Vec<u8>>) {
        (
            self.descriptor,
            self.exchange_ref,
            self.finish_authorization,
        )
    }
}

fn reply_to_assembly_error(reply: ProviderReply) -> PostgresEvmWalletError {
    match reply {
        ProviderReply::Unavailable => PostgresEvmWalletError::Unavailable,
        ProviderReply::Rejected | ProviderReply::Integrity => PostgresEvmWalletError::FenceRejected,
        _ => PostgresEvmWalletError::InvalidAuthority,
    }
}

fn decode_fixed_opaque(value: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(value).map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    bytes
        .try_into()
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)
}

pub(crate) struct OfflineActivationVerifier {
    exact_attestation: WalletNonceDomainActivationAttestation,
    store_incarnation: WalletNonceStoreIncarnation,
    provider_fence_head_ref: EvmWalletReference,
    client: WalletAuthorityProviderClient,
}

impl OfflineActivationVerifier {
    pub(crate) fn verify<'a>(
        &self,
        attestation: &'a WalletNonceDomainActivationAttestation,
    ) -> Result<VerifiedDomainActivation<'a>> {
        attestation
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        if attestation != &self.exact_attestation
            || attestation
                .current_schema_record
                .wallet_nonce_store_lineage_id
                != self.store_incarnation.wallet_nonce_store_lineage_id
            || attestation.initial_store_incarnation_ref
                != self.exact_attestation.initial_store_incarnation_ref
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(VerifiedDomainActivation { attestation })
    }

    pub(crate) const fn exact_attestation(&self) -> &WalletNonceDomainActivationAttestation {
        &self.exact_attestation
    }

    pub(crate) const fn provider_fence_head_ref(&self) -> &EvmWalletReference {
        &self.provider_fence_head_ref
    }

    pub(crate) fn verify_persisted_mutation(
        &self,
        proof: &str,
        schema_name: &str,
        database_oid: u32,
        operation_key: &str,
        mutation: &ProviderMutation,
    ) -> Result<ProviderTargetContext> {
        let context =
            self.client
                .verify_persisted_mutation(proof, None, operation_key, mutation)?;
        // Historical wallet rows survive an authorized physical promotion. The
        // provider proof must therefore belong to this store lineage and an
        // already qualified writer epoch, while the signed context still
        // identifies the exact target that prepared the mutation.
        validate_retained_mutation_target(
            &context,
            &self.store_incarnation,
            schema_name,
            database_oid,
        )?;
        Ok(context)
    }

    pub(crate) async fn begin_read(&self) -> Result<PendingReadLease> {
        let request = ProviderRequest::BeginRead {
            registry_issuance_ref: self.exact_attestation.registry_issuance_ref.clone(),
            store_incarnation: self.store_incarnation.clone(),
            provider_fence_head_ref: self.provider_fence_head_ref.clone(),
        };
        let mut channel = self.client.channel().await?;
        channel.send(&request).await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::LeaseChallenge {
                marker,
                provider_attestation,
            } if valid_marker(&marker) => {
                let provider_attestation = Zeroizing::new(provider_attestation);
                self.client.verify_assertion(
                    &channel,
                    "begin-read",
                    &(
                        &self.exact_attestation.registry_issuance_ref,
                        &self.store_incarnation,
                        &self.provider_fence_head_ref,
                        &marker,
                    ),
                    &provider_attestation,
                )?;
                Ok(PendingReadLease { channel, marker })
            }
            ProviderReply::Unavailable => Err(PostgresEvmWalletError::Unavailable),
            ProviderReply::Rejected | ProviderReply::Integrity => {
                Err(PostgresEvmWalletError::FenceRejected)
            }
            _ => Err(PostgresEvmWalletError::InvalidAuthority),
        }
    }

    pub(crate) async fn begin_write(
        &self,
        state_input_ref: &LexicalValueRef,
        operation_key: &str,
    ) -> Result<PendingWriteLease> {
        if operation_key.is_empty() || operation_key.len() > 512 {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        let request = ProviderRequest::BeginWrite {
            registry_issuance_ref: self.exact_attestation.registry_issuance_ref.clone(),
            store_incarnation: self.store_incarnation.clone(),
            provider_fence_head_ref: self.provider_fence_head_ref.clone(),
            state_input_ref: state_input_ref.clone(),
            operation_key: operation_key.to_owned(),
        };
        let mut channel = self.client.channel().await?;
        channel.send(&request).await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::LeaseChallenge {
                marker,
                provider_attestation,
            } if valid_marker(&marker) => {
                let provider_attestation = Zeroizing::new(provider_attestation);
                self.client.verify_assertion(
                    &channel,
                    "begin-write",
                    &(
                        &self.exact_attestation.registry_issuance_ref,
                        &self.store_incarnation,
                        &self.provider_fence_head_ref,
                        state_input_ref,
                        operation_key,
                        &marker,
                    ),
                    &provider_attestation,
                )?;
                Ok(PendingWriteLease {
                    channel,
                    marker,
                    operation_key: operation_key.to_owned(),
                    state_input_ref: state_input_ref.clone(),
                })
            }
            ProviderReply::Unavailable => Err(PostgresEvmWalletError::Unavailable),
            ProviderReply::Rejected | ProviderReply::Integrity => {
                Err(PostgresEvmWalletError::FenceRejected)
            }
            _ => Err(PostgresEvmWalletError::InvalidAuthority),
        }
    }
}

pub(crate) struct VerifiedDomainActivation<'a> {
    pub(crate) attestation: &'a WalletNonceDomainActivationAttestation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderTargetContext {
    pub(crate) database_oid: u32,
    pub(crate) backend_pid: i32,
    pub(crate) transaction_id: Option<u32>,
    pub(crate) snapshot_id: Option<String>,
    pub(crate) schema_name: String,
    pub(crate) application_marker: String,
    pub(crate) store_incarnation: WalletNonceStoreIncarnation,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedMutationProof {
    provider_id: String,
    challenge: String,
    context: ProviderTargetContext,
    operation_key: String,
    payload_digest: String,
    signature: String,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
// The enclosing provider request boxes this flat authenticated wire value; boxing individual
// fields would add mutation-path allocations without reducing the owning protocol frame.
#[allow(clippy::large_enum_variant)]
pub(crate) enum ProviderMutation {
    Reservation {
        request: ReserveEvmNonceRequest,
        reservation: ReservedWalletNonce,
        state_input_ref: LexicalValueRef,
    },
    CandidateActivation {
        request: ActivateEvmCandidateRequest,
        candidate: ActiveWalletCandidate,
        state_input_ref: LexicalValueRef,
    },
    Completion {
        request: CompleteEvmNonceRequest,
        completion: CompletedWalletNonce,
        state_input_ref: LexicalValueRef,
    },
}

pub(crate) enum ProviderDisposition<T> {
    Current(T),
    Superseded {
        evidence: WalletNonceStoreLineageHead,
        public_head: Box<HistoryObject>,
    },
    Unavailable,
    EntryUnknown,
    Integrity,
}

pub(crate) struct PendingReadLease {
    channel: ProviderChannel,
    marker: String,
}

impl PendingReadLease {
    pub(crate) fn application_marker(&self) -> &str {
        &self.marker
    }

    pub(crate) async fn bind(
        mut self,
        context: ProviderTargetContext,
    ) -> Result<ProviderDisposition<ReadSnapshotLease>> {
        if context.application_marker != self.marker
            || context.transaction_id.is_some()
            || context.snapshot_id.as_deref().is_none_or(str::is_empty)
        {
            return Ok(ProviderDisposition::Integrity);
        }
        self.channel
            .send(&ProviderRequest::BindRead {
                context: context.clone(),
            })
            .await?;
        match self.channel.receive::<ProviderReply>().await? {
            ProviderReply::Current { target_attestation } => {
                let target_attestation = Zeroizing::new(target_attestation);
                verify_channel_assertion(
                    &self.channel,
                    "bind-read",
                    &context,
                    &target_attestation,
                )?;
                Ok(ProviderDisposition::Current(ReadSnapshotLease {
                    channel: Some(self.channel),
                    context,
                    resolution: false,
                }))
            }
            reply => disposition_without_value(reply),
        }
    }
}

pub(crate) struct ReadSnapshotLease {
    channel: Option<ProviderChannel>,
    context: ProviderTargetContext,
    resolution: bool,
}

impl ReadSnapshotLease {
    pub(crate) async fn finish(mut self) -> Result<ProviderDisposition<()>> {
        let mut channel = self
            .channel
            .take()
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        if self.resolution {
            channel.send(&ProviderRequest::FinishResolution).await?;
        } else {
            channel.send(&ProviderRequest::FinishRead).await?;
        }
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::Finished {
                provider_attestation,
            } => {
                let provider_attestation = Zeroizing::new(provider_attestation);
                let assertion_kind = if self.resolution {
                    "finish-resolution"
                } else {
                    "finish-read"
                };
                verify_channel_assertion(
                    &channel,
                    assertion_kind,
                    &self.context,
                    &provider_attestation,
                )?;
                Ok(ProviderDisposition::Current(()))
            }
            reply => disposition_without_value(reply),
        }
    }
}

impl fmt::Debug for ReadSnapshotLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReadSnapshotLease([affine provider authority])")
    }
}

pub(crate) struct PendingWriteLease {
    channel: ProviderChannel,
    marker: String,
    operation_key: String,
    state_input_ref: LexicalValueRef,
}

impl PendingWriteLease {
    pub(crate) fn application_marker(&self) -> &str {
        &self.marker
    }

    pub(crate) async fn bind(
        mut self,
        context: ProviderTargetContext,
    ) -> Result<ProviderDisposition<WriteTransactionLease>> {
        if context.application_marker != self.marker
            || context.transaction_id.is_none()
            || context.snapshot_id.is_some()
        {
            return Ok(ProviderDisposition::Integrity);
        }
        self.channel
            .send(&ProviderRequest::BindWrite {
                context: context.clone(),
                operation_key: self.operation_key.clone(),
            })
            .await?;
        match self.channel.receive::<ProviderReply>().await? {
            ProviderReply::Current { target_attestation } => {
                let target_attestation = Zeroizing::new(target_attestation);
                verify_channel_assertion(
                    &self.channel,
                    "bind-write",
                    &(&context, &self.operation_key),
                    &target_attestation,
                )?;
                Ok(ProviderDisposition::Current(WriteTransactionLease {
                    channel: Some(self.channel),
                    context,
                    operation_key: self.operation_key,
                    state_input_ref: self.state_input_ref,
                    revalidated: false,
                    mutation_prepared: false,
                }))
            }
            reply => disposition_without_value(reply),
        }
    }
}

pub(crate) struct WriteTransactionLease {
    channel: Option<ProviderChannel>,
    context: ProviderTargetContext,
    operation_key: String,
    state_input_ref: LexicalValueRef,
    revalidated: bool,
    mutation_prepared: bool,
}

impl WriteTransactionLease {
    pub(crate) async fn revalidate(&mut self) -> Result<ProviderDisposition<()>> {
        if self.revalidated {
            return Ok(ProviderDisposition::Integrity);
        }
        let channel = self
            .channel
            .as_mut()
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        channel
            .send(&ProviderRequest::RevalidateWrite {
                context: self.context.clone(),
                operation_key: self.operation_key.clone(),
            })
            .await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::Current { target_attestation } => {
                let target_attestation = Zeroizing::new(target_attestation);
                verify_channel_assertion(
                    channel,
                    "revalidate-write",
                    &(&self.context, &self.operation_key),
                    &target_attestation,
                )?;
                self.revalidated = true;
                Ok(ProviderDisposition::Current(()))
            }
            reply => disposition_without_value(reply),
        }
    }

    pub(crate) async fn prepare_mutation(
        &mut self,
        mutation: Box<ProviderMutation>,
    ) -> Result<ProviderDisposition<String>> {
        if !self.revalidated || self.mutation_prepared {
            return Ok(ProviderDisposition::Integrity);
        }
        let channel = self
            .channel
            .as_mut()
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        channel
            .send(&ProviderRequest::PrepareMutation {
                context: self.context.clone(),
                operation_key: self.operation_key.clone(),
                mutation: mutation.clone(),
            })
            .await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::MutationPrepared {
                provider_attestation,
            } => {
                verify_persisted_mutation_proof(
                    &channel.public_key,
                    &channel.provider_id,
                    &provider_attestation,
                    Some(&self.context),
                    &self.operation_key,
                    mutation.as_ref(),
                )?;
                self.mutation_prepared = true;
                Ok(ProviderDisposition::Current(provider_attestation))
            }
            reply => disposition_without_value(reply),
        }
    }

    pub(crate) async fn finish(
        mut self,
        database_commit_observed: bool,
    ) -> Result<ProviderDisposition<()>> {
        let mut channel = self
            .channel
            .take()
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        channel
            .send(&ProviderRequest::FinishWrite {
                database_commit_observed,
                operation_key: self.operation_key.clone(),
            })
            .await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::Finished {
                provider_attestation,
            } => {
                let provider_attestation = Zeroizing::new(provider_attestation);
                verify_channel_assertion(
                    &channel,
                    "finish-write",
                    &(&self.context, &self.operation_key, database_commit_observed),
                    &provider_attestation,
                )?;
                Ok(ProviderDisposition::Current(()))
            }
            reply => disposition_without_value(reply),
        }
    }

    pub(crate) async fn begin_resolution(
        mut self,
    ) -> Result<ProviderDisposition<PendingResolutionLease>> {
        let mut channel = self
            .channel
            .take()
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        channel
            .send(&ProviderRequest::BeginResolution {
                context: self.context.clone(),
                operation_key: self.operation_key.clone(),
            })
            .await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::LeaseChallenge {
                marker,
                provider_attestation,
            } if valid_marker(&marker) => {
                let provider_attestation = Zeroizing::new(provider_attestation);
                verify_channel_assertion(
                    &channel,
                    "begin-resolution",
                    &(&self.context, &self.operation_key, &marker),
                    &provider_attestation,
                )?;
                Ok(ProviderDisposition::Current(PendingResolutionLease {
                    channel,
                    marker,
                    operation_key: self.operation_key,
                    state_input_ref: self.state_input_ref,
                }))
            }
            reply => disposition_without_value(reply),
        }
    }
}

impl fmt::Debug for WriteTransactionLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WriteTransactionLease([affine provider authority])")
    }
}

pub(crate) struct PendingResolutionLease {
    channel: ProviderChannel,
    marker: String,
    operation_key: String,
    state_input_ref: LexicalValueRef,
}

impl PendingResolutionLease {
    pub(crate) const fn state_input_ref(&self) -> &LexicalValueRef {
        &self.state_input_ref
    }
}

impl PendingResolutionLease {
    pub(crate) fn application_marker(&self) -> &str {
        &self.marker
    }

    pub(crate) async fn bind(
        mut self,
        context: ProviderTargetContext,
    ) -> Result<ProviderDisposition<ReadSnapshotLease>> {
        if context.application_marker != self.marker
            || context.transaction_id.is_some()
            || context.snapshot_id.as_deref().is_none_or(str::is_empty)
        {
            return Ok(ProviderDisposition::Integrity);
        }
        self.channel
            .send(&ProviderRequest::BindResolution {
                context: context.clone(),
                operation_key: self.operation_key.clone(),
            })
            .await?;
        match self.channel.receive::<ProviderReply>().await? {
            ProviderReply::Current { target_attestation } => {
                let target_attestation = Zeroizing::new(target_attestation);
                verify_channel_assertion(
                    &self.channel,
                    "bind-resolution",
                    &(&context, &self.operation_key),
                    &target_attestation,
                )?;
                Ok(ProviderDisposition::Current(ReadSnapshotLease {
                    channel: Some(self.channel),
                    context,
                    resolution: true,
                }))
            }
            reply => disposition_without_value(reply),
        }
    }
}

pub(crate) struct ProviderRegistryMaintenance {
    client: WalletAuthorityProviderClient,
    registry_lineage_ref: EvmWalletReference,
    provider_fence_head_ref: EvmWalletReference,
}

impl ProviderRegistryMaintenance {
    pub(crate) const fn registry_lineage_ref(&self) -> &EvmWalletReference {
        &self.registry_lineage_ref
    }

    pub(crate) async fn prepare_issuance(
        &self,
        record: &WalletNonceDomainActivationRecord,
        observed_incarnation: &WalletNonceStoreIncarnation,
        context: ProviderTargetContext,
    ) -> Result<RegistryIssuanceLease> {
        let request = ProviderRequest::PrepareIssuance {
            record: record.clone(),
            observed_incarnation: observed_incarnation.clone(),
            context: context.clone(),
            registry_lineage_ref: self.registry_lineage_ref.clone(),
            provider_fence_head_ref: self.provider_fence_head_ref.clone(),
        };
        let mut channel = self.client.channel().await?;
        channel.send(&request).await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::IssuancePrepared {
                registry_issuance_ref,
                provider_attestation,
            } => {
                let provider_attestation = Zeroizing::new(provider_attestation);
                self.client.verify_assertion(
                    &channel,
                    "prepare-issuance",
                    &(
                        record,
                        observed_incarnation,
                        &context,
                        &self.registry_lineage_ref,
                        &self.provider_fence_head_ref,
                        &registry_issuance_ref,
                    ),
                    &provider_attestation,
                )?;
                registry_issuance_ref
                    .to_content_ref()
                    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
                Ok(RegistryIssuanceLease {
                    channel: Some(channel),
                    registry_issuance_ref,
                    record: record.clone(),
                    observed_incarnation: observed_incarnation.clone(),
                    context,
                })
            }
            ProviderReply::Unavailable => Err(PostgresEvmWalletError::Unavailable),
            ProviderReply::Conflict => Err(PostgresEvmWalletError::PermanentConflict),
            ProviderReply::Rejected => Err(PostgresEvmWalletError::FenceRejected),
            ProviderReply::Integrity => Err(PostgresEvmWalletError::InvalidAuthority),
            _ => Err(PostgresEvmWalletError::InvalidAuthority),
        }
    }

    pub(crate) async fn prepare_promotion(
        &self,
        successor: &WalletNonceStoreSuccessor,
        context: ProviderTargetContext,
    ) -> Result<RegistryPromotionLease> {
        successor
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let request = ProviderRequest::PreparePromotion {
            successor: successor.clone(),
            context: context.clone(),
            registry_lineage_ref: self.registry_lineage_ref.clone(),
            provider_fence_head_ref: self.provider_fence_head_ref.clone(),
        };
        let mut channel = self.client.channel().await?;
        channel.send(&request).await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::PromotionPrepared {
                provider_attestation,
            } => {
                let provider_attestation = Zeroizing::new(provider_attestation);
                self.client.verify_assertion(
                    &channel,
                    "prepare-promotion",
                    &(
                        successor,
                        &context,
                        &self.registry_lineage_ref,
                        &self.provider_fence_head_ref,
                    ),
                    &provider_attestation,
                )?;
                Ok(RegistryPromotionLease {
                    channel: Some(channel),
                    successor: successor.clone(),
                    context,
                })
            }
            ProviderReply::Unavailable => Err(PostgresEvmWalletError::Unavailable),
            ProviderReply::Rejected => Err(PostgresEvmWalletError::FenceRejected),
            ProviderReply::Integrity => Err(PostgresEvmWalletError::InvalidAuthority),
            _ => Err(PostgresEvmWalletError::InvalidAuthority),
        }
    }
}

pub(crate) struct RegistryIssuanceLease {
    channel: Option<ProviderChannel>,
    registry_issuance_ref: EvmWalletReference,
    record: WalletNonceDomainActivationRecord,
    observed_incarnation: WalletNonceStoreIncarnation,
    context: ProviderTargetContext,
}

impl RegistryIssuanceLease {
    pub(crate) const fn registry_issuance_ref(&self) -> &EvmWalletReference {
        &self.registry_issuance_ref
    }

    pub(crate) async fn confirm(
        mut self,
        attestation: &WalletNonceDomainActivationAttestation,
    ) -> Result<()> {
        let mut channel = self
            .channel
            .take()
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        channel
            .send(&ProviderRequest::ConfirmIssuance {
                attestation: attestation.clone(),
            })
            .await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::Confirmed {
                provider_attestation,
            } => {
                let provider_attestation = Zeroizing::new(provider_attestation);
                verify_channel_assertion(
                    &channel,
                    "confirm-issuance",
                    &(
                        &self.record,
                        &self.observed_incarnation,
                        &self.context,
                        attestation,
                    ),
                    &provider_attestation,
                )?;
                Ok(())
            }
            ProviderReply::Unavailable => Err(PostgresEvmWalletError::Unavailable),
            ProviderReply::Rejected => Err(PostgresEvmWalletError::FenceRejected),
            _ => Err(PostgresEvmWalletError::InvalidAuthority),
        }
    }
}

pub(crate) struct RegistryPromotionLease {
    channel: Option<ProviderChannel>,
    successor: WalletNonceStoreSuccessor,
    context: ProviderTargetContext,
}

impl RegistryPromotionLease {
    pub(crate) async fn confirm<T: Serialize>(mut self, attestation: &T) -> Result<()> {
        let mut channel = self
            .channel
            .take()
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        let attestation_json = serde_json::to_value(attestation)
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        channel
            .send(&ProviderRequest::ConfirmPromotion {
                attestation: attestation_json.clone(),
            })
            .await?;
        match channel.receive::<ProviderReply>().await? {
            ProviderReply::Confirmed {
                provider_attestation,
            } => {
                let provider_attestation = Zeroizing::new(provider_attestation);
                verify_channel_assertion(
                    &channel,
                    "confirm-promotion",
                    &(&self.successor, &self.context, &attestation_json),
                    &provider_attestation,
                )?;
                Ok(())
            }
            ProviderReply::Unavailable => Err(PostgresEvmWalletError::Unavailable),
            ProviderReply::Rejected => Err(PostgresEvmWalletError::FenceRejected),
            _ => Err(PostgresEvmWalletError::InvalidAuthority),
        }
    }
}

struct ProviderChannel {
    read: BufReader<ReadHalf<UnixStream>>,
    write: WriteHalf<UnixStream>,
    provider_id: String,
    authentication_challenge: [u8; 32],
    public_key: [u8; 32],
}

impl ProviderChannel {
    async fn send<T: Serialize>(&mut self, value: &T) -> Result<()> {
        let encoded = Zeroizing::new(
            serde_json::to_vec(value).map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        );
        if encoded.len() > MAX_MESSAGE_BYTES || encoded.contains(&b'\n') {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        timeout(PROVIDER_IO_TIMEOUT, async {
            self.write.write_all(&encoded).await?;
            self.write.write_all(b"\n").await?;
            self.write.flush().await
        })
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?
        .map_err(|_| PostgresEvmWalletError::Unavailable)
    }

    async fn receive<T: DeserializeOwned>(&mut self) -> Result<T> {
        let encoded = timeout(
            PROVIDER_IO_TIMEOUT,
            read_bounded_frame(&mut self.read, MAX_MESSAGE_BYTES),
        )
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        serde_json::from_slice(&encoded).map_err(|_| PostgresEvmWalletError::InvalidAuthority)
    }
}

async fn read_bounded_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    maximum_payload_bytes: usize,
) -> std::io::Result<Zeroizing<Vec<u8>>> {
    let mut encoded = Zeroizing::new(Vec::new());
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "provider frame ended before its delimiter",
            ));
        }
        if let Some(delimiter) = available.iter().position(|byte| *byte == b'\n') {
            if encoded.len().saturating_add(delimiter) > maximum_payload_bytes {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "provider frame exceeds its byte bound",
                ));
            }
            encoded.extend_from_slice(&available[..delimiter]);
            reader.consume(delimiter + 1);
            return Ok(encoded);
        }
        if encoded.len().saturating_add(available.len()) > maximum_payload_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "provider frame exceeds its byte bound",
            ));
        }
        encoded.extend_from_slice(available);
        let consumed = available.len();
        reader.consume(consumed);
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProviderRequest {
    Authenticate {
        version: u16,
        provider_id: String,
        fence_lineage_ref: EvmWalletReference,
        chain_registry_lineage_ref: EvmWalletReference,
        current_chain_registry_head_ref: EvmWalletReference,
        challenge: String,
    },
    QualifyRoutingCatalog {
        descriptor: EvmRoutingCatalogDescriptor,
        current_chain_registry_head_ref: EvmWalletReference,
    },
    BeginDeploymentAssembly {
        descriptor: EvmRoutingCatalogDescriptor,
        provider_fence_head_ref: EvmWalletReference,
        binding: DeploymentAssemblyBinding,
    },
    FinishDeploymentAssembly {
        descriptor: EvmRoutingCatalogDescriptor,
        provider_fence_head_ref: EvmWalletReference,
        binding: DeploymentAssemblyBinding,
        assembly_lease: String,
        checkpoint: String,
        exchange_ref: String,
        proofs: Vec<DeploymentAssemblyRouteProofWire>,
    },
    QualifyDomain {
        attestation: WalletNonceDomainActivationAttestation,
        store_incarnation: WalletNonceStoreIncarnation,
        current_public_lineage_head: HistoryObject,
    },
    BeginRead {
        registry_issuance_ref: EvmWalletReference,
        store_incarnation: WalletNonceStoreIncarnation,
        provider_fence_head_ref: EvmWalletReference,
    },
    BindRead {
        context: ProviderTargetContext,
    },
    FinishRead,
    BeginResolution {
        context: ProviderTargetContext,
        operation_key: String,
    },
    BindResolution {
        context: ProviderTargetContext,
        operation_key: String,
    },
    FinishResolution,
    BeginWrite {
        registry_issuance_ref: EvmWalletReference,
        store_incarnation: WalletNonceStoreIncarnation,
        provider_fence_head_ref: EvmWalletReference,
        state_input_ref: LexicalValueRef,
        operation_key: String,
    },
    BindWrite {
        context: ProviderTargetContext,
        operation_key: String,
    },
    RevalidateWrite {
        context: ProviderTargetContext,
        operation_key: String,
    },
    PrepareMutation {
        context: ProviderTargetContext,
        operation_key: String,
        mutation: Box<ProviderMutation>,
    },
    FinishWrite {
        database_commit_observed: bool,
        operation_key: String,
    },
    OpenRegistryMaintenance,
    PrepareIssuance {
        record: WalletNonceDomainActivationRecord,
        observed_incarnation: WalletNonceStoreIncarnation,
        context: ProviderTargetContext,
        registry_lineage_ref: EvmWalletReference,
        provider_fence_head_ref: EvmWalletReference,
    },
    ConfirmIssuance {
        attestation: WalletNonceDomainActivationAttestation,
    },
    PreparePromotion {
        successor: WalletNonceStoreSuccessor,
        context: ProviderTargetContext,
        registry_lineage_ref: EvmWalletReference,
        provider_fence_head_ref: EvmWalletReference,
    },
    ConfirmPromotion {
        attestation: serde_json::Value,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ProviderReply {
    Authenticated {
        signature: String,
    },
    DomainQualified {
        provider_fence_head_ref: EvmWalletReference,
        target_attestation: String,
    },
    RoutingCatalogQualified {
        chain_registry_head_ref: EvmWalletReference,
        provider_fence_head_ref: EvmWalletReference,
        target_attestation: String,
    },
    DeploymentAssemblyBegun {
        assembly_lease: String,
        checkpoint: String,
        finish_authorization_commitment: String,
        route_challenges: Vec<DeploymentAssemblyRouteChallengeWire>,
        provider_attestation: String,
    },
    DeploymentAssemblyFinished {
        finish_authorization: Zeroizing<String>,
        provider_attestation: String,
    },
    RegistryMaintenanceOpened {
        registry_lineage_ref: EvmWalletReference,
        provider_fence_head_ref: EvmWalletReference,
        provider_attestation: String,
    },
    LeaseChallenge {
        marker: String,
        provider_attestation: String,
    },
    Current {
        target_attestation: String,
    },
    MutationPrepared {
        provider_attestation: String,
    },
    Superseded {
        evidence: WalletNonceStoreLineageHead,
        public_head: HistoryObject,
    },
    Finished {
        provider_attestation: String,
    },
    IssuancePrepared {
        registry_issuance_ref: EvmWalletReference,
        provider_attestation: String,
    },
    PromotionPrepared {
        provider_attestation: String,
    },
    Confirmed {
        provider_attestation: String,
    },
    Unavailable,
    EntryUnknown,
    Conflict,
    Integrity,
    Rejected,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentAssemblyRouteChallengeWire {
    route_generation_ref: ContentRef,
    target_identity: String,
    route_challenge: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentAssemblyRouteProofWire {
    ordinal: u16,
    route_generation_ref: ContentRef,
    proof: String,
}

fn disposition_without_value<T>(reply: ProviderReply) -> Result<ProviderDisposition<T>> {
    match reply {
        ProviderReply::Superseded {
            evidence,
            public_head,
        } => Ok(ProviderDisposition::Superseded {
            evidence,
            public_head: Box::new(public_head),
        }),
        ProviderReply::Unavailable => Ok(ProviderDisposition::Unavailable),
        ProviderReply::EntryUnknown => Ok(ProviderDisposition::EntryUnknown),
        ProviderReply::Integrity | ProviderReply::Rejected => Ok(ProviderDisposition::Integrity),
        _ => Err(PostgresEvmWalletError::InvalidAuthority),
    }
}

fn valid_provider_attestation(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROVIDER_PROOF_BYTES
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn validate_finish_authorization(value: &[u8], expected_digest: &[u8; 32]) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_FINISH_AUTHORIZATION_BYTES
        || mfm_canonical::sha256_digest_bytes(value).as_bytes() != expected_digest
    {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

fn decode_persisted_mutation_proof(value: &str) -> Result<PersistedMutationProof> {
    if !valid_provider_attestation(value) {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    let proof: PersistedMutationProof =
        serde_json::from_str(value).map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if serde_json::to_string(&proof).map_or(true, |encoded| encoded != value) {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(proof)
}

pub(crate) async fn verify_historical_incarnation(
    connection: &mut PgConnection,
    incarnation: &WalletNonceStoreIncarnation,
) -> Result<()> {
    let incarnation_ref = canonical_wallet_reference(incarnation)
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let retained_json = sqlx::query_scalar::<_, String>(
        "SELECT incarnation_json FROM wallet_store_incarnations \
         WHERE wallet_nonce_store_lineage_id = $1 \
           AND writer_epoch = $2::numeric \
           AND incarnation_ref = $3",
    )
    .bind(&incarnation.wallet_nonce_store_lineage_id)
    .bind(incarnation.writer_epoch.to_string())
    .bind(reference_text(&incarnation_ref)?)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?
    .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
    let retained: WalletNonceStoreIncarnation = decode_canonical(&retained_json)?;
    retained
        .validate()
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let retained_ref = canonical_wallet_reference(&retained)
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if retained != *incarnation
        || retained_ref != incarnation_ref
        || canonical_json(&retained)? != retained_json
    {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

fn validate_persisted_mutation_context(context: &ProviderTargetContext) -> Result<()> {
    if context.database_oid == 0
        || context.backend_pid <= 0
        || context.transaction_id.is_none()
        || context.snapshot_id.is_some()
        || !valid_marker(&context.application_marker)
    {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    context
        .store_incarnation
        .validate()
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)
}

fn validate_retained_mutation_target(
    context: &ProviderTargetContext,
    expected_store_incarnation: &WalletNonceStoreIncarnation,
    expected_schema: &str,
    expected_database_oid: u32,
) -> Result<()> {
    if context.schema_name != expected_schema
        || context.database_oid != expected_database_oid
        || context.store_incarnation.wallet_nonce_store_lineage_id
            != expected_store_incarnation.wallet_nonce_store_lineage_id
        || context.store_incarnation.writer_epoch > expected_store_incarnation.writer_epoch
    {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

fn verify_persisted_mutation_proof(
    public_key: &[u8; 32],
    expected_provider_id: &str,
    proof_value: &str,
    expected_context: Option<&ProviderTargetContext>,
    expected_operation_key: &str,
    mutation: &ProviderMutation,
) -> Result<ProviderTargetContext> {
    let proof = decode_persisted_mutation_proof(proof_value)?;
    if proof.provider_id != expected_provider_id || proof.operation_key != expected_operation_key {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    if expected_context.is_some_and(|expected| proof.context != *expected) {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    validate_persisted_mutation_context(&proof.context)?;
    let payload_digest = mfm_journal::structured::domain_content_digest(
        "mfm.wallet-authority-provider.assertion-payload.v1",
        &(&proof.context, &proof.operation_key, mutation),
    )
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if proof.payload_digest != payload_digest.as_str() {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    let challenge =
        hex::decode(&proof.challenge).map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if challenge.len() != 32 {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    let signature = decode_signature(&proof.signature)?;
    let mut signed = Vec::with_capacity(
        ASSERTION_DOMAIN.len()
            + challenge.len()
            + "prepare-mutation".len()
            + proof.payload_digest.len(),
    );
    signed.extend_from_slice(ASSERTION_DOMAIN);
    signed.extend_from_slice(&challenge);
    signed.extend_from_slice(b"prepare-mutation");
    signed.extend_from_slice(proof.payload_digest.as_bytes());
    UnparsedPublicKey::new(&ED25519, *public_key)
        .verify(&signed, signature.as_ref())
        .map_err(|_| PostgresEvmWalletError::FenceRejected)?;
    Ok(proof.context)
}

fn decode_signature(signature: &str) -> Result<Zeroizing<[u8; 64]>> {
    let mut decoded = Zeroizing::new([0_u8; 64]);
    hex::decode_to_slice(signature, decoded.as_mut())
        .map_err(|_| PostgresEvmWalletError::FenceRejected)?;
    Ok(decoded)
}

fn references_are_distinct(references: &[&EvmWalletReference]) -> bool {
    references
        .iter()
        .enumerate()
        .all(|(index, reference)| references[..index].iter().all(|prior| *prior != *reference))
}

fn routing_catalog_contains_authority_reference(
    descriptor: &EvmRoutingCatalogDescriptor,
    reference: &EvmWalletReference,
) -> bool {
    descriptor.chain_registry_head_ref() == reference
        || descriptor.chain_instances().iter().any(|attestation| {
            attestation.declaration().chain_registry_lineage_ref() == reference
                || attestation.registry_head_ref_at_issuance() == reference
                || attestation.registry_issuance_ref() == reference
        })
        || descriptor
            .generations()
            .iter()
            .any(|generation| generation.route_membership_issuance_ref() == reference)
}

fn valid_marker(marker: &str) -> bool {
    marker.len() == 62 && marker.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn fresh_application_marker() -> Result<String> {
    // PostgreSQL truncates `application_name` after 63 bytes. Keep the
    // authenticated marker below that boundary while retaining 248 bits of
    // entropy.
    let mut marker = [0_u8; 31];
    SystemRandom::new()
        .fill(&mut marker)
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    Ok(hex::encode(marker))
}

pub(crate) fn target_session_marker_lock_keys(marker: &str) -> Result<(i32, i32)> {
    if !valid_marker(marker) {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    let first = u32::from_str_radix(&marker[..8], 16)
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let second = u32::from_str_radix(&marker[8..16], 16)
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    Ok((first as i32, second as i32))
}

fn verify_channel_assertion<T: Serialize>(
    channel: &ProviderChannel,
    assertion_kind: &str,
    payload: &T,
    signature_hex: &str,
) -> Result<()> {
    let digest = mfm_journal::structured::domain_content_digest(
        "mfm.wallet-authority-provider.assertion-payload.v1",
        payload,
    )
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let signature = decode_signature(signature_hex)?;
    let mut signed = Vec::with_capacity(
        ASSERTION_DOMAIN.len()
            + channel.authentication_challenge.len()
            + assertion_kind.len()
            + digest.as_str().len(),
    );
    signed.extend_from_slice(ASSERTION_DOMAIN);
    signed.extend_from_slice(&channel.authentication_challenge);
    signed.extend_from_slice(assertion_kind.as_bytes());
    signed.extend_from_slice(digest.as_str().as_bytes());
    let verified = UnparsedPublicKey::new(&ED25519, channel.public_key)
        .verify(&signed, signature.as_ref())
        .map_err(|_| PostgresEvmWalletError::FenceRejected);
    verified
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[tokio::test]
    async fn client_frame_reader_accepts_exact_message_budget() {
        let payload = vec![b'x'; MAX_MESSAGE_BYTES - 1];
        let (mut peer, client) = tokio::io::duplex(payload.len() + 1);
        let writer = tokio::spawn(async move {
            peer.write_all(&payload).await.expect("exact payload");
            peer.write_all(b"\n").await.expect("frame delimiter");
        });
        let mut reader = BufReader::new(client);
        let frame = read_bounded_frame(&mut reader, MAX_MESSAGE_BYTES)
            .await
            .expect("exact message budget is accepted");
        assert_eq!(frame.len(), MAX_MESSAGE_BYTES - 1);
        writer.await.expect("hostile writer");
    }

    #[tokio::test]
    async fn client_frame_reader_rejects_oversized_and_unterminated_peers() {
        for payload in [
            vec![b'x'; MAX_MESSAGE_BYTES + 1],
            b"unterminated-provider-reply".to_vec(),
        ] {
            let (mut peer, client) = tokio::io::duplex(payload.len() + 1);
            let writer = tokio::spawn(async move {
                peer.write_all(&payload).await.expect("hostile payload");
            });
            let mut reader = BufReader::new(client);
            let error = read_bounded_frame(&mut reader, MAX_MESSAGE_BYTES)
                .await
                .expect_err("hostile peer frame must fail");
            assert!(matches!(
                error.kind(),
                std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
            ));
            writer.await.expect("hostile writer");
        }
    }

    #[test]
    fn provider_attestation_accepts_exact_budget_and_rejects_one_byte_over() {
        let exact = "x".repeat(MAX_DEPLOYMENT_PROOF_BYTES);
        assert!(valid_provider_attestation(&exact));

        let one_byte_over = format!("{exact}x");
        assert!(!valid_provider_attestation(&one_byte_over));
    }

    #[test]
    fn finish_authorization_accepts_exact_budget_and_rejects_one_byte_over() {
        let exact = vec![b'x'; MAX_FINISH_AUTHORIZATION_BYTES];
        let exact_digest = *mfm_canonical::sha256_digest_bytes(&exact).as_bytes();
        validate_finish_authorization(&exact, &exact_digest)
            .expect("exact finish-authorization budget is accepted");

        let one_byte_over = vec![b'x'; MAX_FINISH_AUTHORIZATION_BYTES + 1];
        let one_byte_over_digest = *mfm_canonical::sha256_digest_bytes(&one_byte_over).as_bytes();
        assert!(validate_finish_authorization(&one_byte_over, &one_byte_over_digest).is_err());
    }

    #[test]
    fn deployment_route_count_accepts_exact_budget_and_rejects_one_route_over() {
        assert!(
            validate_deployment_route_count(MAX_DEPLOYMENT_ROUTES, MAX_DEPLOYMENT_ROUTES).is_ok()
        );
        assert!(validate_deployment_route_count(
            MAX_DEPLOYMENT_ROUTES + 1,
            MAX_DEPLOYMENT_ROUTES + 1
        )
        .is_err());
    }

    #[test]
    fn deployment_route_proof_accepts_exact_budget_and_rejects_one_byte_over() {
        let route_generation_ref = ContentRef::new(
            mfm_ids::SchemaId::new(
                "mfm.test.provider-route-generation",
                "1",
                mfm_ids::DigestAlgorithm::Sha256JcsV1,
                mfm_canonical::sha256_digest_bytes(b"mfm.test.provider-route-generation"),
            )
            .expect("route schema"),
            mfm_ids::ContentDigest::from_digest(
                mfm_ids::DigestAlgorithm::Sha256V1,
                mfm_canonical::sha256_digest_bytes(b"mfm.test.provider-route-generation.ref"),
            ),
        )
        .expect("route generation reference");
        DeploymentAssemblyRouteProof::new(
            0,
            route_generation_ref.clone(),
            vec![b'x'; MAX_DEPLOYMENT_PROOF_BYTES].into_boxed_slice(),
        )
        .expect("exact deployment proof budget is accepted");
        assert!(DeploymentAssemblyRouteProof::new(
            0,
            route_generation_ref,
            vec![b'x'; MAX_DEPLOYMENT_PROOF_BYTES + 1].into_boxed_slice(),
        )
        .is_err());
    }
}

#[cfg(test)]
mod mutation_proof_tests {
    use super::*;

    use alloy_primitives::{Address, B256, U256};
    use mfm_evm::{
        canonical_wallet_reference, derive_authenticated_intent_issuer_id,
        derive_evm_candidate_operation_key, derive_evm_chain_lineage_id,
        derive_evm_nonce_completion_key, derive_evm_nonce_reservation_key,
        derive_exact_candidate_activation_permit, derive_submission_intent_id,
        derive_submission_semantics_digest, derive_wallet_nonce_domain,
        evm_submission_expansion_policy_ref, evm_wallet_assurance_policy_ref,
        evm_wallet_nonce_policy_ref, ActivateEvmCandidateRequest, ActiveWalletCandidate,
        AttestedWalletCandidate, CanonicalTerminalOutcome, ChainInstanceDeclaration,
        ChainInstanceRegistryAttestation, CompleteEvmNonceRequest, CompletedWalletNonce,
        EvmCandidateFamily, EvmFinalizedHeadObservation, EvmInclusionBlockObservation,
        EvmReceiptLookupObservation, EvmRoutingGenerationRef, EvmTransactionIntent,
        EvmTransactionLookupObservation, EvmTransactionTarget, EvmWalletFeeCandidate,
        EvmWalletTransactionAction, EvmWalletTransactionTemplate, ExclusiveCurrentControl,
        ExecutionDisposition, ObservedPendingNonceFloor, PriorEffectDisposition,
        PriorResourceDisposition, QualifiedPendingNonceFloor, ReplayExclusionDisposition,
        ReserveEvmNonceRequest, ReservedWalletNonce, TerminalWitnesses, TransactionNonce,
        UnsignedWalletCandidate, WalletNonceDomainActivationAttestation,
        WalletNonceDomainActivationRecord, WalletNonceStoreIncarnation,
    };
    use mfm_ids::{StableId, TenantScopeId};
    use mfm_journal::structured::{domain_content_digest, LexicalValueRef, TypedValueRef};
    use ring::signature::{Ed25519KeyPair, KeyPair};

    const SIGNING_SEED: [u8; 32] = [0x42; 32];
    const WRONG_SIGNING_SEED: [u8; 32] = [0x24; 32];

    struct CompletionProofFixture {
        mutation: ProviderMutation,
        completion: CompletedWalletNonce,
        state_input: LexicalValueRef,
        context: ProviderTargetContext,
        operation_key: String,
        provider_id: String,
    }

    fn completion_fixture() -> CompletionProofFixture {
        let common_ref = evm_wallet_nonce_policy_ref().expect("nonce policy reference");
        let declaration = ChainInstanceDeclaration::new(
            common_ref.clone(),
            StableId::new("mfm.evm.fixture/chain-instance").expect("chain instance id"),
            1,
            B256::repeat_byte(0x11),
            U256::from(10_u64),
            B256::repeat_byte(0x12),
        )
        .expect("chain declaration");
        let chain_attestation = ChainInstanceRegistryAttestation::new(
            declaration,
            common_ref.clone(),
            common_ref.clone(),
        )
        .expect("chain attestation");
        let chain_binding = chain_attestation.binding().expect("chain binding");
        let chain_lineage =
            derive_evm_chain_lineage_id(chain_binding.qualified_chain_instance_id())
                .expect("chain lineage");
        let sender = Address::repeat_byte(0x21);
        let nonce_domain = derive_wallet_nonce_domain(chain_lineage, sender).expect("nonce domain");
        let route_generation = EvmRoutingGenerationRef::from_content_ref(
            common_ref
                .to_content_ref()
                .expect("route content reference"),
        )
        .expect("route generation");
        let sender_inventory = domain_content_digest(
            "mfm.evm.fixture-sender-path-inventory.v1",
            &(nonce_domain.clone(), sender),
        )
        .expect("sender inventory digest");
        let activation_record = WalletNonceDomainActivationRecord {
            activation_contract_ref: common_ref.clone(),
            qualified_activation_registry_lineage_ref: common_ref.clone(),
            wallet_nonce_store_lineage_id: "mfm.evm.fixture/wallet-store".to_owned(),
            initial_store_incarnation_ref: common_ref.clone(),
            wallet_nonce_domain: nonce_domain.clone(),
            chain_instance_attestation: chain_attestation,
            initial_route_generation_ref: route_generation,
            initial_route_membership_issuance_ref: common_ref.clone(),
            sender_identity: format!("{sender:#x}"),
            issuer_namespace_contract_ref: common_ref.clone(),
            replay_exclusion_contract_ref: common_ref.clone(),
            replay_exclusion_disposition:
                ReplayExclusionDisposition::EveryPriorRequestReplayAndRetryIngressExcluded,
            finalized_sender_nonce_floor: 0,
            finalized_block_number: "10".to_owned(),
            finalized_block_hash: format!("{:#x}", B256::repeat_byte(0x12)),
            qualified_observation_proof_ref: common_ref.clone(),
            exhaustive_sender_path_inventory_digest: sender_inventory.as_str().to_owned(),
            exclusive_current_control:
                ExclusiveCurrentControl::EveryPriorWriterSignerRelayerOperatorStaleDeploymentAndDirectSubmitPathFenced,
            prior_effect_disposition: PriorEffectDisposition::NoUnresolvedPossibleEntry,
            prior_resource_disposition:
                PriorResourceDisposition::EveryPriorAllocationAndSubmittedCandidateTerminal,
            new_idempotency_epoch: "mfm.evm.fixture/idempotency-epoch".to_owned(),
        };
        activation_record.validate().expect("activation record");
        let activation_record_ref =
            canonical_wallet_reference(&activation_record).expect("activation record reference");
        let activation = WalletNonceDomainActivationAttestation {
            activation_record_ref,
            registry_issuance_ref: common_ref.clone(),
            activation_registry_lineage_ref: common_ref.clone(),
            initial_store_incarnation_ref: common_ref.clone(),
            current_schema_record: activation_record,
        };
        activation.validate().expect("activation attestation");

        let assurance_ref = evm_wallet_assurance_policy_ref().expect("assurance policy");
        let template = EvmWalletTransactionTemplate::new(
            EvmTransactionTarget::new("mutation-proof-target").expect("transaction target"),
            EvmWalletTransactionAction::call(Address::repeat_byte(0x31)),
            U256::ZERO,
            [],
            Vec::new(),
            U256::from(21_000_u64),
        )
        .expect("transaction template");
        let intent = EvmTransactionIntent::new(
            chain_binding,
            nonce_domain.clone(),
            template,
            StableId::new("mfm.evm.fixture/semantic-signer").expect("signer id"),
            common_ref.clone(),
            common_ref.clone(),
            assurance_ref,
        )
        .expect("transaction intent");
        let candidate_family = EvmCandidateFamily::new(
            &intent,
            vec![
                EvmWalletFeeCandidate::new(U256::from(100_u64), U256::from(2_u64))
                    .expect("candidate fee"),
            ],
        )
        .expect("candidate family");
        let tenant = TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000051")
            .expect("tenant scope");
        let principal = StableId::new("mfm.evm.fixture/principal").expect("principal");
        let issuer = derive_authenticated_intent_issuer_id(&tenant, &principal, &common_ref)
            .expect("intent issuer");
        let submission_intent_id =
            derive_submission_intent_id(&nonce_domain, &issuer, "mutation-proof-token")
                .expect("submission intent");
        let route_generation_ref = common_ref.clone();
        let expansion_ref = evm_submission_expansion_policy_ref().expect("expansion policy");
        let submission_semantics_digest = derive_submission_semantics_digest(
            &intent,
            &candidate_family,
            1,
            &expansion_ref,
            &route_generation_ref,
            &activation,
            &common_ref,
        )
        .expect("submission semantics");
        let reservation_key =
            derive_evm_nonce_reservation_key(&nonce_domain, &submission_intent_id)
                .expect("reservation key");
        let qualified_floor = QualifiedPendingNonceFloor {
            observed: ObservedPendingNonceFloor {
                nonce_domain: nonce_domain.clone(),
                route_generation_ref: route_generation_ref.clone(),
                pending_nonce: TransactionNonce::new(0).expect("pending nonce"),
            },
            pending_floor_policy_ref: common_ref.clone(),
        };
        let content_ref = common_ref
            .to_content_ref()
            .expect("state content reference");
        let state_input = LexicalValueRef::new(
            content_ref.clone(),
            TypedValueRef {
                contract_ref: content_ref.clone(),
                value_ref: content_ref,
            },
        );
        let reserve_request = ReserveEvmNonceRequest {
            nonce_domain: nonce_domain.clone(),
            domain_activation_attestation: activation.clone(),
            issuer_namespace_contract_ref: common_ref.clone(),
            submission_intent_id: submission_intent_id.clone(),
            submission_semantics_digest: submission_semantics_digest.clone(),
            transaction_intent: intent.clone(),
            candidate_family: candidate_family.clone(),
            observation_rounds: 1,
            reservation_key: reservation_key.clone(),
            qualified_floor: qualified_floor.clone(),
        };
        let observed_floor_ref = domain_content_digest(
            "mfm.evm.wallet-observed-floor-provenance.v1",
            &(&reserve_request.qualified_floor, &state_input),
        )
        .expect("observed-floor digest")
        .as_str()
        .to_owned();
        let mut reservation = ReservedWalletNonce {
            nonce_domain: nonce_domain.clone(),
            domain_activation_record_ref: activation.activation_record_ref.clone(),
            nonce: 0,
            semantic_reservation_key: reservation_key.clone(),
            submission_intent_id: submission_intent_id.clone(),
            transaction_intent_digest: intent.digest().to_owned(),
            candidate_family_ref: candidate_family.digest().to_owned(),
            observed_floor_ref,
            resource_lineage_ref: common_ref.clone(),
            reservation_evidence_ref: common_ref.clone(),
        };
        let reservation_evidence = domain_content_digest(
            "mfm.evm.wallet-reservation-evidence.v1",
            &(
                &reserve_request,
                &state_input,
                &reservation.resource_lineage_ref,
                reservation.nonce,
            ),
        )
        .expect("reservation evidence digest");
        reservation.reservation_evidence_ref = common_ref
            .with_content_digest(reservation_evidence)
            .expect("reservation evidence reference");

        let unsigned_envelope = intent
            .unsigned_candidate(reservation.nonce, &candidate_family.candidates()[0])
            .expect("unsigned candidate");
        let unsigned = UnsignedWalletCandidate {
            transaction_intent: intent.clone(),
            semantic_reservation_key: reservation.semantic_reservation_key.clone(),
            nonce: reservation.nonce,
            candidate_ordinal: 0,
            fee: candidate_family.candidates()[0].clone(),
            unsigned_candidate_digest: format!("{:#x}", unsigned_envelope.signing_digest()),
        };
        let attested = AttestedWalletCandidate {
            semantic_reservation_key: reservation.semantic_reservation_key.clone(),
            candidate_ordinal: 0,
            candidate_descriptor_ref: canonical_wallet_reference(&unsigned)
                .expect("candidate descriptor"),
            unsigned_candidate_digest: unsigned.unsigned_candidate_digest.clone(),
            transaction_hash: format!("{:#x}", B256::repeat_byte(0x41)),
            semantic_signer_id: intent.semantic_signer_id().to_owned(),
            signing_profile_contract_ref: common_ref.clone(),
            signer_attestation_ref: common_ref.clone(),
        };
        let activation_request = ActivateEvmCandidateRequest {
            nonce_domain: nonce_domain.clone(),
            candidate_operation_key: derive_evm_candidate_operation_key(
                &reservation.semantic_reservation_key,
                0,
            )
            .expect("candidate operation key"),
            next_candidate: attested.clone(),
            activation_permit: derive_exact_candidate_activation_permit(&reservation, &[], 0, 0)
                .expect("initial activation permit"),
        };
        let activation_evidence = domain_content_digest(
            "mfm.evm.wallet-candidate-activation-evidence.v1",
            &(
                &activation_request,
                &state_input,
                &reservation.resource_lineage_ref,
            ),
        )
        .expect("activation evidence digest");
        let activation_evidence_ref = common_ref
            .with_content_digest(activation_evidence)
            .expect("activation evidence reference");
        let active_candidate = ActiveWalletCandidate {
            attested_candidate: attested,
            activation_evidence_ref: activation_evidence_ref.clone(),
            provider_activation_attestation: "fixture-provider-attestation".to_owned(),
        };
        let transaction_hash = active_candidate.attested_candidate.transaction_hash.clone();
        let inclusion_block_hash = format!("{:#x}", B256::repeat_byte(0x42));
        let canonical_public_result = "{\"execution_disposition\":\"succeeded\"}".to_owned();
        let canonical_terminal_outcome = CanonicalTerminalOutcome {
            nonce_domain: nonce_domain.clone(),
            semantic_reservation_key: reservation.semantic_reservation_key.clone(),
            submission_intent_id: reservation.submission_intent_id.clone(),
            transaction_intent_digest: reservation.transaction_intent_digest.clone(),
            nonce: reservation.nonce,
            winning_candidate_ordinal: 0,
            winning_activation_evidence_ref: activation_evidence_ref,
            transaction_hash: transaction_hash.clone(),
            inclusion_block_number: "10".to_owned(),
            inclusion_block_hash: inclusion_block_hash.clone(),
            terminal_assurance_contract_ref: evm_wallet_assurance_policy_ref()
                .expect("assurance policy"),
            execution_disposition: ExecutionDisposition::Succeeded,
            canonical_public_result: canonical_public_result.clone(),
        };
        let terminal_witnesses = TerminalWitnesses {
            transaction: EvmTransactionLookupObservation::Found {
                transaction_hash: transaction_hash.clone(),
                block_number: Some("10".to_owned()),
                block_hash: Some(inclusion_block_hash.clone()),
            },
            receipt: EvmReceiptLookupObservation::Found {
                transaction_hash: transaction_hash.clone(),
                block_number: "10".to_owned(),
                block_hash: inclusion_block_hash.clone(),
                status: 1,
            },
            finalized_head: EvmFinalizedHeadObservation {
                block_number: "11".to_owned(),
                block_hash: format!("{:#x}", B256::repeat_byte(0x43)),
            },
            inclusion_block: EvmInclusionBlockObservation {
                block_number: "10".to_owned(),
                block_hash: inclusion_block_hash,
            },
            terminal_assurance_contract_ref: canonical_terminal_outcome
                .terminal_assurance_contract_ref
                .clone(),
            canonical_public_result: canonical_public_result.clone(),
        };
        terminal_witnesses
            .validate_against(&canonical_terminal_outcome)
            .expect("terminal witnesses");
        let completion_request = CompleteEvmNonceRequest {
            nonce_domain: nonce_domain.clone(),
            completion_key: derive_evm_nonce_completion_key(&reservation.semantic_reservation_key)
                .expect("completion key"),
            current_reservation: reservation.clone(),
            canonical_terminal_outcome,
            terminal_witnesses,
        };
        let completion_evidence = domain_content_digest(
            "mfm.evm.wallet-completion-evidence.v1",
            &(
                &completion_request,
                &state_input,
                &reservation.resource_lineage_ref,
            ),
        )
        .expect("completion evidence digest");
        let completion_evidence_ref = common_ref
            .with_content_digest(completion_evidence)
            .expect("completion evidence reference");
        let completion = CompletedWalletNonce::with_recovery_closure(
            completion_request.nonce_domain.clone(),
            completion_request.current_reservation.nonce,
            completion_request
                .current_reservation
                .semantic_reservation_key
                .clone(),
            completion_request.completion_key.clone(),
            completion_request.canonical_terminal_outcome.clone(),
            completion_request.terminal_witnesses.clone(),
            vec![active_candidate],
            canonical_wallet_reference(&completion_request.terminal_witnesses)
                .expect("terminal witness reference")
                .content_digest()
                .to_owned(),
            completion_evidence_ref,
            "fixture-provider-attestation".to_owned(),
            reservation,
            intent,
            candidate_family,
            activation,
            qualified_floor,
            route_generation_ref,
            common_ref.clone(),
            1,
            submission_semantics_digest,
            reserve_request,
            completion_request.clone(),
            vec![activation_request],
            vec![state_input.clone()],
            state_input.clone(),
            state_input.clone(),
        )
        .expect("completion recovery closure");
        completion.validate().expect("completion validation");
        let completion_preimage = completion
            .provider_mutation_preimage()
            .expect("completion provider preimage");
        let mutation = ProviderMutation::Completion {
            request: completion_request.clone(),
            completion: completion_preimage,
            state_input_ref: state_input.clone(),
        };
        let store_incarnation = WalletNonceStoreIncarnation {
            wallet_nonce_store_lineage_id: "mfm.evm.fixture/wallet-store".to_owned(),
            writer_epoch: 1,
            physical_target_instance_id: "mfm.evm.fixture/target".to_owned(),
            non_exportable_target_public_key_ref: common_ref.clone(),
            target_attestation_contract_ref: common_ref,
        };
        store_incarnation.validate().expect("store incarnation");
        let context = ProviderTargetContext {
            database_oid: 17,
            backend_pid: 23,
            transaction_id: Some(29),
            snapshot_id: None,
            schema_name: "fixture_schema".to_owned(),
            application_marker: "a".repeat(62),
            store_incarnation,
        };
        let operation_key = completion_request.completion_key.as_str().to_owned();
        CompletionProofFixture {
            mutation,
            completion,
            state_input,
            context,
            operation_key,
            provider_id: "mfm.evm.fixture/provider".to_owned(),
        }
    }

    fn public_key(seed: &[u8; 32]) -> [u8; 32] {
        Ed25519KeyPair::from_seed_unchecked(seed)
            .expect("deterministic signing seed")
            .public_key()
            .as_ref()
            .try_into()
            .expect("Ed25519 public key length")
    }

    fn persisted_proof(
        context: &ProviderTargetContext,
        operation_key: &str,
        provider_id: &str,
        mutation: &ProviderMutation,
        seed: &[u8; 32],
    ) -> String {
        let payload_digest = domain_content_digest(
            "mfm.wallet-authority-provider.assertion-payload.v1",
            &(context, operation_key, mutation),
        )
        .expect("provider payload digest");
        let challenge = [0x33_u8; 32];
        let mut signed = Vec::with_capacity(
            ASSERTION_DOMAIN.len()
                + challenge.len()
                + "prepare-mutation".len()
                + payload_digest.as_str().len(),
        );
        signed.extend_from_slice(ASSERTION_DOMAIN);
        signed.extend_from_slice(&challenge);
        signed.extend_from_slice(b"prepare-mutation");
        signed.extend_from_slice(payload_digest.as_str().as_bytes());
        let signature = Ed25519KeyPair::from_seed_unchecked(seed)
            .expect("deterministic signing seed")
            .sign(&signed);
        serde_json::to_string(&PersistedMutationProof {
            provider_id: provider_id.to_owned(),
            challenge: hex::encode(challenge),
            context: context.clone(),
            operation_key: operation_key.to_owned(),
            payload_digest: payload_digest.as_str().to_owned(),
            signature: hex::encode(signature.as_ref()),
        })
        .expect("canonical persisted proof")
    }

    #[test]
    fn detached_completion_proof_verifies_and_rehydrates_without_io() {
        let fixture = completion_fixture();
        let proof = persisted_proof(
            &fixture.context,
            &fixture.operation_key,
            &fixture.provider_id,
            &fixture.mutation,
            &SIGNING_SEED,
        );
        let verified_context = verify_persisted_mutation_proof(
            &public_key(&SIGNING_SEED),
            &fixture.provider_id,
            &proof,
            Some(&fixture.context),
            &fixture.operation_key,
            &fixture.mutation,
        )
        .expect("detached completion proof");
        assert_eq!(verified_context, fixture.context);
        assert_eq!(
            fixture.state_input,
            match &fixture.mutation {
                ProviderMutation::Completion {
                    state_input_ref, ..
                } => state_input_ref.clone(),
                _ => panic!("completion mutation fixture"),
            }
        );
        let rehydrated =
            CompletedWalletNonce::from_recovery_closure(&fixture.completion.recovery_closure)
                .expect("rehydrate completion closure");
        assert_eq!(rehydrated, fixture.completion);
        assert_eq!(
            CompletedWalletNonce::project_public_result_from_recovery_closure(
                &fixture.completion.recovery_closure,
            )
            .expect("project public completion"),
            fixture
                .completion
                .canonical_terminal_outcome
                .canonical_public_result,
        );
    }

    #[test]
    fn detached_completion_proof_rejects_crypto_and_target_substitutions() {
        let fixture = completion_fixture();
        let valid = persisted_proof(
            &fixture.context,
            &fixture.operation_key,
            &fixture.provider_id,
            &fixture.mutation,
            &SIGNING_SEED,
        );
        let mut forged_signature = decode_persisted_mutation_proof(&valid).expect("valid proof");
        forged_signature.signature.replace_range(0..2, "00");
        let forged_signature = serde_json::to_string(&forged_signature).expect("forged proof");
        assert!(matches!(
            verify_persisted_mutation_proof(
                &public_key(&SIGNING_SEED),
                &fixture.provider_id,
                &forged_signature,
                None,
                &fixture.operation_key,
                &fixture.mutation,
            ),
            Err(PostgresEvmWalletError::FenceRejected)
        ));
        assert!(matches!(
            verify_persisted_mutation_proof(
                &public_key(&WRONG_SIGNING_SEED),
                &fixture.provider_id,
                &valid,
                None,
                &fixture.operation_key,
                &fixture.mutation,
            ),
            Err(PostgresEvmWalletError::FenceRejected)
        ));

        let mut tampered_mutation = fixture.mutation.clone();
        if let ProviderMutation::Completion { request, .. } = &mut tampered_mutation {
            request.canonical_terminal_outcome.canonical_public_result =
                "{\"execution_disposition\":\"reverted\"}".to_owned();
        }
        assert!(matches!(
            verify_persisted_mutation_proof(
                &public_key(&SIGNING_SEED),
                &fixture.provider_id,
                &valid,
                None,
                &fixture.operation_key,
                &tampered_mutation,
            ),
            Err(PostgresEvmWalletError::InvalidAuthority)
        ));

        let mut substituted_provider =
            decode_persisted_mutation_proof(&valid).expect("valid proof");
        substituted_provider.provider_id = "mfm.evm.fixture/other-provider".to_owned();
        let substituted_provider =
            serde_json::to_string(&substituted_provider).expect("provider substitution");
        assert!(matches!(
            verify_persisted_mutation_proof(
                &public_key(&SIGNING_SEED),
                &fixture.provider_id,
                &substituted_provider,
                None,
                &fixture.operation_key,
                &fixture.mutation,
            ),
            Err(PostgresEvmWalletError::InvalidAuthority)
        ));

        let mut substituted_operation =
            decode_persisted_mutation_proof(&valid).expect("valid proof");
        substituted_operation.operation_key = "mfm.evm.fixture/other-operation".to_owned();
        let substituted_operation =
            serde_json::to_string(&substituted_operation).expect("operation substitution");
        assert!(matches!(
            verify_persisted_mutation_proof(
                &public_key(&SIGNING_SEED),
                &fixture.provider_id,
                &substituted_operation,
                None,
                &fixture.operation_key,
                &fixture.mutation,
            ),
            Err(PostgresEvmWalletError::InvalidAuthority)
        ));

        let mut substituted_context = fixture.context.clone();
        substituted_context.schema_name = "other_schema".to_owned();
        let substituted_context_proof = persisted_proof(
            &substituted_context,
            &fixture.operation_key,
            &fixture.provider_id,
            &fixture.mutation,
            &SIGNING_SEED,
        );
        assert!(validate_retained_mutation_target(
            &decode_persisted_mutation_proof(&substituted_context_proof)
                .expect("substituted context proof")
                .context,
            &fixture.context.store_incarnation,
            &fixture.context.schema_name,
            fixture.context.database_oid,
        )
        .is_err());
        assert!(verify_persisted_mutation_proof(
            &public_key(&SIGNING_SEED),
            &fixture.provider_id,
            &substituted_context_proof,
            Some(&fixture.context),
            &fixture.operation_key,
            &fixture.mutation,
        )
        .is_err());

        for substitution in [
            |context: &mut ProviderTargetContext| context.database_oid = 18,
            |context: &mut ProviderTargetContext| {
                context.store_incarnation.wallet_nonce_store_lineage_id =
                    "mfm.evm.fixture/other-lineage".to_owned()
            },
            |context: &mut ProviderTargetContext| context.store_incarnation.writer_epoch = 2,
        ] {
            let mut context = fixture.context.clone();
            substitution(&mut context);
            let proof = persisted_proof(
                &context,
                &fixture.operation_key,
                &fixture.provider_id,
                &fixture.mutation,
                &SIGNING_SEED,
            );
            let parsed = decode_persisted_mutation_proof(&proof).expect("substitution proof");
            assert!(validate_retained_mutation_target(
                &parsed.context,
                &fixture.context.store_incarnation,
                &fixture.context.schema_name,
                fixture.context.database_oid,
            )
            .is_err());
        }
    }

    #[test]
    fn detached_completion_proof_rejects_digest_challenge_and_canonicality_tampering() {
        let fixture = completion_fixture();
        let valid = persisted_proof(
            &fixture.context,
            &fixture.operation_key,
            &fixture.provider_id,
            &fixture.mutation,
            &SIGNING_SEED,
        );
        let mut bad_digest = decode_persisted_mutation_proof(&valid).expect("valid proof");
        bad_digest.payload_digest =
            "sha256v1:0000000000000000000000000000000000000000000000000000000000000000".to_owned();
        let bad_digest = serde_json::to_string(&bad_digest).expect("bad digest proof");
        assert!(matches!(
            verify_persisted_mutation_proof(
                &public_key(&SIGNING_SEED),
                &fixture.provider_id,
                &bad_digest,
                None,
                &fixture.operation_key,
                &fixture.mutation,
            ),
            Err(PostgresEvmWalletError::InvalidAuthority)
        ));

        for challenge in ["not-hex", "00", &"00".repeat(33)] {
            let mut forged = decode_persisted_mutation_proof(&valid).expect("valid proof");
            forged.challenge = challenge.to_owned();
            let forged = serde_json::to_string(&forged).expect("challenge proof");
            assert!(matches!(
                verify_persisted_mutation_proof(
                    &public_key(&SIGNING_SEED),
                    &fixture.provider_id,
                    &forged,
                    None,
                    &fixture.operation_key,
                    &fixture.mutation,
                ),
                Err(PostgresEvmWalletError::InvalidAuthority)
            ));
        }

        let unknown = valid.replacen(
            "{\"provider_id\"",
            "{\"unknown\":\"field\",\"provider_id\"",
            1,
        );
        assert!(decode_persisted_mutation_proof(&unknown).is_err());
        let duplicate = valid.replacen(
            "\"provider_id\":",
            "\"provider_id\":\"mfm.evm.fixture/duplicate\",\"provider_id\":",
            1,
        );
        assert!(decode_persisted_mutation_proof(&duplicate).is_err());
        let parsed: serde_json::Value = serde_json::from_str(&valid).expect("valid JSON");
        let reordered = serde_json::to_string(&parsed).expect("reordered JSON");
        assert_ne!(reordered, valid);
        assert!(decode_persisted_mutation_proof(&reordered).is_err());
        assert!(decode_persisted_mutation_proof(&format!(" {valid} ")).is_err());
        assert!(decode_persisted_mutation_proof("not-json").is_err());
        assert!(decode_persisted_mutation_proof("é").is_err());
        assert!(
            decode_persisted_mutation_proof(&"x".repeat(MAX_PROVIDER_PROOF_BYTES + 1)).is_err()
        );
    }
}
