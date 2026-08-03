use std::sync::Arc;

use async_trait::async_trait;
use mfm_ids::{InvocationIdentity, RunId, StableId, StoreScopeId, TenantScopeId};
use mfm_journal::structured::{
    HistoryObject, PriorRunFactSourceManifest, ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
    ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_storage_evm_postgres::QualifiedEvmRoutingCatalog;

use crate::{
    AccessAuditPage, AccessTarget, AdmitRunRequest, AdmitRunResponse, DriveResponse,
    EntryPointContract, ErrorClass, ExportRequest, ExportedRun, PageRequest, PublicError,
    PublicRunView, ReplayRequest, ReplayResponse, RunAccessGrant, RunAccessPolicy,
    SecretCredential, TransitionTracePage,
};

/// Complete public physical-release material consumed by EVM deployment assembly.
///
/// This aggregate contains no live transport, signer handle, wallet authority,
/// endpoint, or credential. Its histories remain stable across compatible
/// physical generations.
pub struct EvmWalletDeploymentReleaseMaterial {
    rpc: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
    signer: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
    broadcast: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
    balance: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
    wallet_read: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
    wallet_effect: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
    broadcast_lineage_head: mfm_evm::BroadcastLineageHead,
    broadcast_lineage_head_object: HistoryObject,
}

impl EvmWalletDeploymentReleaseMaterial {
    /// Collects the exact retained histories and current broadcast lineage head.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rpc: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
        signer: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
        broadcast: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
        balance: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
        wallet_read: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
        wallet_effect: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
        broadcast_lineage_head: mfm_evm::BroadcastLineageHead,
        broadcast_lineage_head_object: HistoryObject,
    ) -> Result<Self, PublicError> {
        if broadcast_lineage_head.validate().is_err()
            || broadcast_lineage_head_object.validate().is_err()
            || broadcast_lineage_head
                .public_lineage_head_ref
                .to_content_ref()
                .ok()
                != Some(broadcast_lineage_head_object.content_ref.clone())
        {
            return Err(wallet_deployment_invalid());
        }
        Ok(Self {
            rpc,
            signer,
            broadcast,
            balance,
            wallet_read,
            wallet_effect,
            broadcast_lineage_head,
            broadcast_lineage_head_object,
        })
    }

    /// Returns the RPC release history.
    pub const fn rpc(&self) -> &mfm_evm_live::EvmPhysicalBindingReleaseHistory {
        &self.rpc
    }

    /// Returns the signer release history.
    pub const fn signer(&self) -> &mfm_evm_live::EvmPhysicalBindingReleaseHistory {
        &self.signer
    }

    /// Returns the broadcast release history.
    pub const fn broadcast(&self) -> &mfm_evm_live::EvmPhysicalBindingReleaseHistory {
        &self.broadcast
    }

    /// Returns the balance release history.
    pub const fn balance(&self) -> &mfm_evm_live::EvmPhysicalBindingReleaseHistory {
        &self.balance
    }

    /// Returns the wallet-read release history.
    pub const fn wallet_read(&self) -> &mfm_evm_live::EvmPhysicalBindingReleaseHistory {
        &self.wallet_read
    }

    /// Returns the wallet-effect release history.
    pub const fn wallet_effect(&self) -> &mfm_evm_live::EvmPhysicalBindingReleaseHistory {
        &self.wallet_effect
    }

    /// Returns the exact current broadcast lineage head.
    pub const fn broadcast_lineage_head(&self) -> &mfm_evm::BroadcastLineageHead {
        &self.broadcast_lineage_head
    }

    /// Returns the public object attesting the current broadcast lineage head.
    pub const fn broadcast_lineage_head_object(&self) -> &HistoryObject {
        &self.broadcast_lineage_head_object
    }
}

/// Affine raw inputs for one provider-controlled EVM wallet deployment assembly.
///
/// No caller-assembled live binding is accepted. The authenticated provider
/// consumes this closure through its begin/finish lease protocol.
///
/// ```compile_fail
/// use std::sync::Arc;
/// use mfm_app::EvmWalletDeploymentAssemblyInput;
/// use mfm_signing::GenerationGuardedDeterministicSigningProvider;
///
/// fn bypass(
///     signer: Arc<dyn GenerationGuardedDeterministicSigningProvider>,
/// ) -> EvmWalletDeploymentAssemblyInput {
///     signer
/// }
/// ```
///
/// ```compile_fail
/// use std::sync::Arc;
/// use mfm_app::EvmWalletDeploymentAssemblyInput;
/// use mfm_evm_live::{
///     EvmStructuredBalanceBindings, EvmStructuredLiveBindings,
///     EvmStructuredWalletBindings,
/// };
///
/// fn bypass(
///     submission: Arc<EvmStructuredLiveBindings>,
///     balance: Arc<EvmStructuredBalanceBindings>,
///     wallet: Arc<EvmStructuredWalletBindings>,
/// ) -> EvmWalletDeploymentAssemblyInput {
///     (submission, balance, wallet)
/// }
/// ```
pub struct EvmWalletDeploymentAssemblyInput {
    routing_manifest: mfm_portfolio::PortfolioRoutingManifest,
    qualified_routing_catalog: QualifiedEvmRoutingCatalog,
    pending_rpc_inventory: mfm_evm_live::transport::PendingEvmRpcInventory,
    wallet_authority: mfm_storage_evm_postgres::PostgresWalletNonceAuthority,
    signer: mfm_keystore::QualifiedKeystoreSigner,
    submission: mfm_evm::EvmSubmissionConfiguration,
    context_manifest: HistoryObject,
    prior_run_source_manifest: HistoryObject,
    portfolio_coverage_inputs: Vec<mfm_evm::EvmBalanceLaneInput>,
    releases: EvmWalletDeploymentReleaseMaterial,
}

impl EvmWalletDeploymentAssemblyInput {
    /// Consumes every concrete authority and public configuration needed by the provider bracket.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        routing_manifest: mfm_portfolio::PortfolioRoutingManifest,
        qualified_routing_catalog: QualifiedEvmRoutingCatalog,
        pending_rpc_inventory: mfm_evm_live::transport::PendingEvmRpcInventory,
        wallet_authority: mfm_storage_evm_postgres::PostgresWalletNonceAuthority,
        signer: mfm_keystore::QualifiedKeystoreSigner,
        submission: mfm_evm::EvmSubmissionConfiguration,
        context_manifest: HistoryObject,
        prior_run_source_manifest: HistoryObject,
        portfolio_coverage_inputs: Vec<mfm_evm::EvmBalanceLaneInput>,
        releases: EvmWalletDeploymentReleaseMaterial,
    ) -> Result<Self, PublicError> {
        use mfm_evm::WalletNonceAuthority as _;

        let catalog = qualified_routing_catalog.descriptor();
        let routing_policy = qualified_routing_catalog
            .routing_policy_object()
            .map_err(|_| wallet_deployment_invalid())?;
        let semantics = submission
            .deployment_semantics(
                signer.semantic_signer_id(),
                signer.binding().expected_public_identity(),
            )
            .map_err(|_| wallet_deployment_invalid())?;
        let route = semantics
            .route_generation_ref()
            .to_content_ref()
            .ok()
            .and_then(|reference| {
                mfm_evm::EvmRoutingGenerationRef::from_content_ref(reference).ok()
            });
        let route_matches = route.as_ref().is_some_and(|reference| {
            catalog
                .resolve_generation(reference)
                .is_some_and(|generation| {
                    generation.chain_instance() == submission.transaction_intent().chain_instance()
                })
        });
        let manifest_matches = routing_manifest
            .evm_routing_bindings()
            .iter()
            .all(|binding| {
                catalog
                    .resolve_generation(binding.routing_generation_ref())
                    .is_some_and(|generation| {
                        generation.network_id() == binding.network_id()
                            && generation.chain_instance() == binding.chain_instance()
                    })
            });
        let coverage_matches = portfolio_coverage_inputs.iter().all(|input| {
            catalog
                .resolve_generation(input.binding().routing_generation_ref())
                .is_some_and(|generation| {
                    generation.network_id() == input.binding().network_id()
                        && generation.chain_instance() == input.binding().chain_instance()
                })
        });
        let signer_contract_matches = signer.semantic_signer_contract_ref()
            == &semantics
                .semantic_signer_contract_ref()
                .to_content_ref()
                .map_err(|_| wallet_deployment_invalid())?;
        let activation_chain_matches = semantics
            .domain_activation_attestation()
            .current_schema_record
            .chain_instance_attestation
            .binding()
            .is_ok_and(|binding| {
                &binding == submission.transaction_intent().chain_instance()
                    && catalog.chain_instances().contains(
                        &semantics
                            .domain_activation_attestation()
                            .current_schema_record
                            .chain_instance_attestation,
                    )
            });
        if routing_policy.validate().is_err()
            || routing_policy.object_type.as_str() != ADMISSION_ROUTING_POLICY_OBJECT_TYPE
            || context_manifest.validate().is_err()
            || PriorRunFactSourceManifest::from_history_object(&prior_run_source_manifest).is_err()
            || context_manifest.object_type.as_str() != ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE
            || !route_matches
            || !manifest_matches
            || !coverage_matches
            || !signer_contract_matches
            || !activation_chain_matches
            || pending_rpc_inventory.routing_catalog_descriptor() != catalog
            || wallet_authority.domain_activation_attestation()
                != semantics.domain_activation_attestation()
            || release_material_matches(
                &releases,
                &routing_policy.content_ref,
                catalog,
                semantics.route_generation_ref(),
                signer.binding().durable_generation_ref(),
                wallet_authority.domain_activation_attestation(),
            )
            .is_err()
        {
            return Err(wallet_deployment_invalid());
        }
        Ok(Self {
            routing_manifest,
            qualified_routing_catalog,
            pending_rpc_inventory,
            wallet_authority,
            signer,
            submission,
            context_manifest,
            prior_run_source_manifest,
            portfolio_coverage_inputs,
            releases,
        })
    }

    /// Returns the exact public portfolio routing manifest.
    pub const fn routing_manifest(&self) -> &mfm_portfolio::PortfolioRoutingManifest {
        &self.routing_manifest
    }

    /// Returns the provider-qualified catalog bearer without releasing ownership.
    pub const fn qualified_routing_catalog(&self) -> &QualifiedEvmRoutingCatalog {
        &self.qualified_routing_catalog
    }

    /// Returns the pending private inventory without releasing its retained routes.
    pub const fn pending_rpc_inventory(&self) -> &mfm_evm_live::transport::PendingEvmRpcInventory {
        &self.pending_rpc_inventory
    }

    /// Returns the concrete provider-qualified wallet authority.
    pub const fn wallet_authority(
        &self,
    ) -> &mfm_storage_evm_postgres::PostgresWalletNonceAuthority {
        &self.wallet_authority
    }

    /// Returns the qualified concrete signer bearer without releasing it.
    pub const fn signer(&self) -> &mfm_keystore::QualifiedKeystoreSigner {
        &self.signer
    }

    /// Returns the exact configured submission semantics.
    pub const fn submission(&self) -> &mfm_evm::EvmSubmissionConfiguration {
        &self.submission
    }

    /// Returns the admission context manifest.
    pub const fn context_manifest(&self) -> &HistoryObject {
        &self.context_manifest
    }

    /// Returns the prior-run source manifest.
    pub const fn prior_run_source_manifest(&self) -> &HistoryObject {
        &self.prior_run_source_manifest
    }

    /// Returns the exact portfolio coverage inputs.
    pub fn portfolio_coverage_inputs(&self) -> &[mfm_evm::EvmBalanceLaneInput] {
        &self.portfolio_coverage_inputs
    }

    /// Returns the complete public release material.
    pub const fn releases(&self) -> &EvmWalletDeploymentReleaseMaterial {
        &self.releases
    }
}

fn release_material_matches(
    releases: &EvmWalletDeploymentReleaseMaterial,
    routing_policy_ref: &mfm_ids::ContentRef,
    catalog: &mfm_evm::EvmRoutingCatalogDescriptor,
    route_generation_ref: &mfm_evm::EvmWalletReference,
    signer_generation_ref: &mfm_ids::ContentRef,
    activation: &mfm_evm::WalletNonceDomainActivationAttestation,
) -> Result<(), PublicError> {
    let histories = [
        &releases.rpc,
        &releases.signer,
        &releases.broadcast,
        &releases.balance,
        &releases.wallet_read,
        &releases.wallet_effect,
    ];
    let route_target_ref = route_generation_ref
        .to_content_ref()
        .map_err(|_| wallet_deployment_invalid())?;
    let catalog_target_ref = catalog
        .content_ref()
        .map_err(|_| wallet_deployment_invalid())?;
    let wallet_target_ref = activation
        .initial_store_incarnation_ref
        .to_content_ref()
        .map_err(|_| wallet_deployment_invalid())?;
    // Every retained wallet release (including successors) must name the
    // authority's actual current physical incarnation, not only the root.
    let wallet_releases_current = releases
        .wallet_read
        .releases()
        .chain(releases.wallet_effect.releases())
        .all(|release| release.physical_target_ref() == &wallet_target_ref);
    if histories
        .iter()
        .any(|history| history.current().admitted_routing_policy_ref() != routing_policy_ref)
        || releases.rpc.current().physical_target_ref() != &route_target_ref
        || releases.signer.current().physical_target_ref() != signer_generation_ref
        || releases.balance.current().physical_target_ref() != &catalog_target_ref
        || !wallet_releases_current
        || releases.broadcast.current().physical_target_ref()
            != &releases.broadcast_lineage_head_object.content_ref
        || releases
            .broadcast
            .current()
            .predecessor_binding_ref()
            .is_some()
            && releases.broadcast.current().activation_lineage_head_ref()
                != Some(&releases.broadcast_lineage_head_object.content_ref)
    {
        return Err(wallet_deployment_invalid());
    }
    Ok(())
}

/// Deployment-owned structured EVM bindings and immutable admission material.
///
/// This value exposes no database pool, registry administration, writer fence,
/// signer secret, or generic invoker. Every live handle is already sealed to
/// its exact public certificates and routing policy.
///
/// ```compile_fail
/// use mfm_app::EvmWalletDeployment;
/// fn duplicate(value: &EvmWalletDeployment) -> EvmWalletDeployment {
///     value.clone()
/// }
/// ```
///
/// ```compile_fail
/// use mfm_app::EvmWalletDeployment;
/// fn forge() -> EvmWalletDeployment {
///     EvmWalletDeployment {}
/// }
/// ```
pub struct EvmWalletDeployment {
    routing_manifest: mfm_portfolio::PortfolioRoutingManifest,
    routing_catalog: mfm_evm::EvmRoutingCatalogDescriptor,
    routing_policy: HistoryObject,
    context_manifest: HistoryObject,
    prior_run_source_manifest: HistoryObject,
    portfolio_coverage_inputs: Vec<mfm_evm::EvmBalanceLaneInput>,
    /// Exact deployment semantics derived from the qualified live signer.
    sealed_submission_semantics: mfm_evm::EvmDeploymentSubmissionSemantics,
    /// Full public signing identity bound at qualification (key + account).
    sealed_public_signing_identity: mfm_signing::PublicSigningIdentity,
    submission_bindings: Arc<mfm_evm_live::EvmStructuredLiveBindings>,
    balance_bindings: Arc<mfm_evm_live::EvmStructuredBalanceBindings>,
    wallet_bindings: Arc<mfm_evm_live::EvmStructuredWalletBindings>,
}

impl EvmWalletDeployment {
    /// Consumes the complete raw authority closure through provider Begin/Finish
    /// and privately constructs the only production live-binding tuple.
    pub async fn assemble(input: EvmWalletDeploymentAssemblyInput) -> Result<Self, PublicError> {
        use mfm_evm::WalletNonceAuthority as _;

        let EvmWalletDeploymentAssemblyInput {
            routing_manifest,
            qualified_routing_catalog,
            pending_rpc_inventory,
            wallet_authority,
            signer,
            submission,
            context_manifest,
            prior_run_source_manifest,
            portfolio_coverage_inputs,
            releases,
        } = input;
        let routing_policy = qualified_routing_catalog
            .routing_policy_object()
            .map_err(|_| wallet_deployment_invalid())?;
        let semantics = submission
            .deployment_semantics(
                signer.semantic_signer_id(),
                signer.binding().expected_public_identity(),
            )
            .map_err(|_| wallet_deployment_invalid())?;
        let semantic_contract_refs = [
            semantics.semantic_signer_contract_ref(),
            semantics.signing_profile_contract_ref(),
            semantics.broadcast_contract_ref(),
            semantics.submission_contract_ref(),
            semantics.expansion_contract_ref(),
            semantics.terminal_assurance_contract_ref(),
        ]
        .into_iter()
        .map(|reference| {
            reference
                .to_content_ref()
                .map_err(|_| wallet_deployment_invalid())
        })
        .collect::<Result<Vec<_>, _>>()?;
        let release_history_digests = [
            &releases.rpc,
            &releases.signer,
            &releases.broadcast,
            &releases.balance,
            &releases.wallet_read,
            &releases.wallet_effect,
        ]
        .into_iter()
        .map(|history| {
            history
                .content_digest()
                .map_err(|_| wallet_deployment_invalid())
        })
        .collect::<Result<Vec<_>, _>>()?;
        let assembly_binding = mfm_storage_evm_postgres::DeploymentAssemblyBinding::new(
            wallet_authority.domain_activation_attestation().clone(),
            wallet_authority.store_incarnation().clone(),
            wallet_authority.current_public_lineage_head().clone(),
            wallet_authority.provider_fence_head_ref().clone(),
            signer.semantic_signer_id().clone(),
            signer.binding().durable_generation_ref().clone(),
            signer.binding().fence_attestation_ref().clone(),
            signer.binding().direct_sign_exclusion_ref().clone(),
            semantic_contract_refs,
            release_history_digests,
        )
        .map_err(|_| wallet_deployment_invalid())?;
        let provider_assembly = qualified_routing_catalog
            .begin_deployment_assembly(assembly_binding)
            .await
            .map_err(|_| wallet_deployment_invalid())?;
        let live_challenges = provider_assembly
            .route_challenges()
            .map(|challenge| {
                mfm_evm_live::transport::EvmRpcInventoryChallenge::new(
                    challenge.route_generation_ref().clone(),
                    mfm_evm_live::transport::EvmRpcTargetIdentity::new(
                        *challenge.target_identity(),
                    ),
                    mfm_evm_live::transport::EvmRpcAssemblyLease::new(
                        *provider_assembly.assembly_lease(),
                    ),
                    mfm_evm_live::transport::EvmRpcInventoryCheckpoint::new(
                        *provider_assembly.checkpoint(),
                    ),
                    mfm_evm_live::transport::EvmRpcRouteChallenge::new(
                        *challenge.route_challenge(),
                    ),
                    *provider_assembly.finish_authorization_commitment(),
                )
            })
            .collect::<Vec<_>>();
        let (pending_rpc_inventory, inventory_proofs) = pending_rpc_inventory
            .exchange(
                mfm_evm_live::transport::EvmRpcInventoryChallenges::new(live_challenges)
                    .map_err(|_| wallet_deployment_invalid())?,
            )
            .await
            .map_err(|_| wallet_deployment_invalid())?;
        let provider_proofs = inventory_proofs
            .iter()
            .map(|proof| {
                mfm_storage_evm_postgres::DeploymentAssemblyRouteProof::new(
                    proof.ordinal(),
                    proof.challenge().route_generation_ref().clone(),
                    proof.proof().to_vec().into_boxed_slice(),
                )
                .map_err(|_| wallet_deployment_invalid())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let exchange_ref = *inventory_proofs.exchange_ref().as_bytes();
        let finished = provider_assembly
            .finish(exchange_ref, provider_proofs)
            .await
            .map_err(|_| wallet_deployment_invalid())?;
        let (routing_catalog, provider_exchange_ref, authorization) =
            finished.into_finish_material();
        if provider_exchange_ref != exchange_ref {
            return Err(wallet_deployment_invalid());
        }
        let completed_inventory = pending_rpc_inventory
            .finish(
                mfm_evm_live::transport::EvmRpcInventoryFinishAuthorization::new(authorization)
                    .map_err(|_| wallet_deployment_invalid())?,
            )
            .map_err(|_| wallet_deployment_invalid())?;
        if completed_inventory.exchange_ref().as_bytes() != &provider_exchange_ref {
            return Err(wallet_deployment_invalid());
        }
        let transport = Arc::new(completed_inventory.into_transport());
        let (semantic_signer_id, semantic_signer_contract_ref, signer_provider) = signer
            .into_read_signing_provider()
            .await
            .map_err(|_| wallet_deployment_invalid())?;
        if &semantic_signer_id != semantics.semantic_signer_id()
            || semantic_signer_contract_ref
                != semantics
                    .semantic_signer_contract_ref()
                    .to_content_ref()
                    .map_err(|_| wallet_deployment_invalid())?
        {
            return Err(wallet_deployment_invalid());
        }
        let sealed_public_signing_identity = signer_provider
            .binding()
            .expected_public_identity()
            .clone();
        let chain_instance = submission.transaction_intent().chain_instance().clone();
        let submission_bindings = Arc::new(
            mfm_evm_live::EvmStructuredLiveBindings::new(
                Arc::clone(&transport),
                chain_instance,
                semantics.route_generation_ref().clone(),
                routing_policy.content_ref.clone(),
                semantic_signer_id,
                semantics.expected_sender(),
                signer_provider,
                releases.rpc,
                releases.signer,
                releases.broadcast,
                releases.broadcast_lineage_head,
                releases.broadcast_lineage_head_object,
            )
            .map_err(|_| wallet_deployment_invalid())?,
        );
        let balance_bindings = Arc::new(
            mfm_evm_live::EvmStructuredBalanceBindings::new(
                Arc::clone(&transport),
                routing_policy.content_ref.clone(),
                releases.balance,
            )
            .map_err(|_| wallet_deployment_invalid())?,
        );
        let wallet_bindings = Arc::new(
            mfm_evm_live::EvmStructuredWalletBindings::new(
                Arc::new(wallet_authority),
                routing_policy.content_ref.clone(),
                releases.wallet_read,
                releases.wallet_effect,
            )
            .map_err(|_| wallet_deployment_invalid())?,
        );
        Ok(Self {
            routing_manifest,
            routing_catalog,
            routing_policy,
            context_manifest,
            prior_run_source_manifest,
            portfolio_coverage_inputs,
            sealed_submission_semantics: semantics,
            sealed_public_signing_identity,
            submission_bindings,
            balance_bindings,
            wallet_bindings,
        })
    }

    pub(crate) fn into_parts(self) -> EvmWalletDeploymentParts {
        EvmWalletDeploymentParts {
            routing_manifest: self.routing_manifest,
            routing_catalog: self.routing_catalog,
            routing_policy: self.routing_policy,
            context_manifest: self.context_manifest,
            prior_run_source_manifest: self.prior_run_source_manifest,
            portfolio_coverage_inputs: self.portfolio_coverage_inputs,
            sealed_submission_semantics: self.sealed_submission_semantics,
            sealed_public_signing_identity: self.sealed_public_signing_identity,
            submission_bindings: self.submission_bindings,
            balance_bindings: self.balance_bindings,
            wallet_bindings: self.wallet_bindings,
        }
    }
}

pub(crate) struct EvmWalletDeploymentParts {
    pub(crate) routing_manifest: mfm_portfolio::PortfolioRoutingManifest,
    pub(crate) routing_catalog: mfm_evm::EvmRoutingCatalogDescriptor,
    pub(crate) routing_policy: HistoryObject,
    pub(crate) context_manifest: HistoryObject,
    pub(crate) prior_run_source_manifest: HistoryObject,
    pub(crate) sealed_submission_semantics: mfm_evm::EvmDeploymentSubmissionSemantics,
    pub(crate) sealed_public_signing_identity: mfm_signing::PublicSigningIdentity,
    pub(crate) portfolio_coverage_inputs: Vec<mfm_evm::EvmBalanceLaneInput>,
    pub(crate) submission_bindings: Arc<mfm_evm_live::EvmStructuredLiveBindings>,
    pub(crate) balance_bindings: Arc<mfm_evm_live::EvmStructuredBalanceBindings>,
    pub(crate) wallet_bindings: Arc<mfm_evm_live::EvmStructuredWalletBindings>,
}

/// Opaque run-facing application facade used by process transports.
///
/// Every protected call consumes one credential and obtains a fresh purpose-specific policy
/// decision. The facade retains no authentication session and exposes no store, runtime, replay,
/// object, or fact service.
#[derive(Clone)]
pub struct Application {
    store_scope_id: StoreScopeId,
    policy: Arc<dyn RunAccessPolicy>,
    entry_points: Arc<[EntryPointContract]>,
    backend: Arc<dyn ApplicationBackend>,
}

impl Application {
    pub(crate) fn new(
        store_scope_id: StoreScopeId,
        policy: Arc<dyn RunAccessPolicy>,
        entry_points: Vec<EntryPointContract>,
        backend: impl ApplicationBackend + 'static,
    ) -> Self {
        Self {
            store_scope_id,
            policy,
            entry_points: entry_points.into(),
            backend: Arc::new(backend),
        }
    }

    /// Returns every complete published entry-point contract.
    ///
    /// Discovery is intentionally unauthenticated. The returned contracts contain no deployment
    /// secrets or caller-specific state.
    pub fn entry_points(&self) -> &[EntryPointContract] {
        &self.entry_points
    }

    /// Checks the authoritative structured run-history store.
    ///
    /// Readiness performs the bounded writable-lineage probe without opening signer or wallet
    /// mutation authority.
    pub async fn check_ready(&self) -> Result<(), PublicError> {
        self.backend.check_ready().await
    }

    /// Authenticates, authorizes, plans, certifies, and admits one exact logical root.
    pub async fn admit_run(
        &self,
        credential: SecretCredential,
        request: AdmitRunRequest,
    ) -> Result<AdmitRunResponse, PublicError> {
        let entry_point = self
            .entry_points()
            .iter()
            .find(|entry| entry.entry_point_id().as_str() == request.entry_point_id().as_str())
            .cloned()
            .ok_or_else(entry_point_not_found)?;
        let configured_target = selected_admission_target(&entry_point, &request)?;
        let target = AccessTarget::AdmitTarget {
            store_scope_id: self.store_scope_id.clone(),
            entry_point_operation_id: entry_point.entry_point_operation_id().clone(),
            configured_target: configured_target.clone(),
            invocation_identity: request.invocation_identity().clone(),
        };
        let authorized = self
            .policy
            .authorize(&credential, RunAccessGrant::Admit, &target)
            .await?;
        let (tenant_scope_id, authenticated_principal_id) = authorized.into_parts();
        let call = AuthorizedAdmissionCall {
            tenant_scope_id,
            authenticated_principal_id,
            entry_point_operation_id: entry_point.entry_point_operation_id().clone(),
            configured_target,
            invocation_identity: request.invocation_identity().clone(),
        };
        self.backend.admit_run(&call, entry_point, request).await
    }

    /// Authenticates, authorizes, and executes at most one legal run action.
    pub async fn drive_once(
        &self,
        credential: SecretCredential,
        run_id: RunId,
    ) -> Result<DriveResponse, PublicError> {
        let call = self
            .authorize_run::<run_grant::Drive>(credential, RunAccessGrant::Drive, run_id)
            .await?;
        self.backend.drive_once(&call).await
    }

    /// Authenticates, authorizes, and reads the sole ordinary public run view.
    pub async fn read_public_run(
        &self,
        credential: SecretCredential,
        run_id: RunId,
    ) -> Result<PublicRunView, PublicError> {
        let call = self
            .authorize_run::<run_grant::ReadPublic>(credential, RunAccessGrant::ReadPublic, run_id)
            .await?;
        self.backend.read_public_run(&call).await
    }

    /// Authenticates, authorizes, and runs callback-free replay orchestration.
    pub async fn replay_run(
        &self,
        credential: SecretCredential,
        run_id: RunId,
        request: ReplayRequest,
    ) -> Result<ReplayResponse, PublicError> {
        let call = self
            .authorize_run::<run_grant::Replay>(credential, RunAccessGrant::Replay, run_id)
            .await?;
        if request.requires_export_authorization() {
            call.authorize_same_run_grant(RunAccessGrant::Export)
                .await?;
        }
        self.backend.replay_run(&call, request).await
    }

    /// Authenticates every source and reads one fixed-head transition-trace page.
    pub async fn read_transition_trace(
        &self,
        credential: SecretCredential,
        run_id: RunId,
        page: PageRequest,
    ) -> Result<TransitionTracePage, PublicError> {
        let call = self
            .authorize_run::<run_grant::InspectTrace>(
                credential,
                RunAccessGrant::InspectTrace,
                run_id,
            )
            .await?;
        self.backend.read_transition_trace(&call, page).await
    }

    /// Authenticates, authorizes, and reads one fixed-head safe access-audit page.
    pub async fn read_access_audit(
        &self,
        credential: SecretCredential,
        run_id: RunId,
        page: PageRequest,
    ) -> Result<AccessAuditPage, PublicError> {
        let call = self
            .authorize_run::<run_grant::InspectAudit>(
                credential,
                RunAccessGrant::InspectAudit,
                run_id,
            )
            .await?;
        self.backend.read_access_audit(&call, page).await
    }

    /// Authenticates every dependency and returns a complete portable-export stream.
    pub async fn export_run(
        &self,
        credential: SecretCredential,
        run_id: RunId,
        request: ExportRequest,
    ) -> Result<ExportedRun, PublicError> {
        let call = self
            .authorize_run::<run_grant::Export>(credential, RunAccessGrant::Export, run_id)
            .await?;
        self.backend.export_run(&call, request).await
    }

    async fn authorize_run<G: run_grant::RunGrantMarker>(
        &self,
        credential: SecretCredential,
        grant: RunAccessGrant,
        run_id: RunId,
    ) -> Result<AuthorizedRunCall<'_, G>, PublicError> {
        debug_assert_eq!(grant, G::GRANT);
        let target = AccessTarget::RunTarget {
            store_scope_id: self.store_scope_id.clone(),
            run_id: run_id.clone(),
        };
        let authorized = self.policy.authorize(&credential, grant, &target).await?;
        let (tenant_scope_id, authenticated_principal_id) = authorized.into_parts();
        Ok(AuthorizedRunCall {
            credential,
            policy: self.policy.as_ref(),
            store_scope_id: &self.store_scope_id,
            tenant_scope_id,
            authenticated_principal_id,
            run_id,
            grant,
            _marker: std::marker::PhantomData,
        })
    }
}

fn selected_admission_target(
    entry_point: &EntryPointContract,
    request: &AdmitRunRequest,
) -> Result<StableId, PublicError> {
    if entry_point.entry_point_id().as_str() == mfm_portfolio::PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID {
        let selector: mfm_portfolio::PortfolioSnapshotSelector =
            serde_json::from_value(request.input().as_json().clone())
                .map_err(|_| admission_request_invalid())?;
        StableId::new(selector.target().as_str()).map_err(|_| admission_request_invalid())
    } else if entry_point.entry_point_id().as_str()
        == mfm_evm::EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID
    {
        let selector: mfm_evm::EvmSubmitTransactionSelector =
            serde_json::from_value(request.input().as_json().clone())
                .map_err(|_| admission_request_invalid())?;
        StableId::new(selector.target().as_str()).map_err(|_| admission_request_invalid())
    } else {
        Err(admission_request_invalid())
    }
}

fn admission_request_invalid() -> PublicError {
    PublicError::bad_request(
        "AdmissionRequestInvalid",
        "Admission request does not match the published entry-point input",
    )
}

fn entry_point_not_found() -> PublicError {
    PublicError::not_found(
        "EntryPointNotFound",
        "The exact entry-point id is not registered",
    )
}

/// Typestate markers for purpose-bound authorized run calls.
pub(crate) mod run_grant {
    use crate::RunAccessGrant;

    /// Closed marker for one approved run-access grant.
    pub(crate) trait RunGrantMarker: Send + Sync + 'static {
        /// Exact grant this marker retains.
        const GRANT: RunAccessGrant;
    }

    /// Drive one legal run action.
    pub(crate) struct Drive;
    /// Read the ordinary public run view.
    pub(crate) struct ReadPublic;
    /// Inspect transition-trace material.
    pub(crate) struct InspectTrace;
    /// Inspect access-audit material.
    pub(crate) struct InspectAudit;
    /// Replay recorded history.
    pub(crate) struct Replay;
    /// Export one portable run stream.
    pub(crate) struct Export;

    impl RunGrantMarker for Drive {
        const GRANT: RunAccessGrant = RunAccessGrant::Drive;
    }
    impl RunGrantMarker for ReadPublic {
        const GRANT: RunAccessGrant = RunAccessGrant::ReadPublic;
    }
    impl RunGrantMarker for InspectTrace {
        const GRANT: RunAccessGrant = RunAccessGrant::InspectTrace;
    }
    impl RunGrantMarker for InspectAudit {
        const GRANT: RunAccessGrant = RunAccessGrant::InspectAudit;
    }
    impl RunGrantMarker for Replay {
        const GRANT: RunAccessGrant = RunAccessGrant::Replay;
    }
    impl RunGrantMarker for Export {
        const GRANT: RunAccessGrant = RunAccessGrant::Export;
    }
}

/// Purpose-bound authorized run call that retains the exact approved grant.
pub(crate) struct AuthorizedRunCall<'policy, G: run_grant::RunGrantMarker> {
    credential: SecretCredential,
    policy: &'policy dyn RunAccessPolicy,
    store_scope_id: &'policy StoreScopeId,
    tenant_scope_id: TenantScopeId,
    authenticated_principal_id: StableId,
    run_id: RunId,
    grant: RunAccessGrant,
    _marker: std::marker::PhantomData<G>,
}

impl<G: run_grant::RunGrantMarker> AuthorizedRunCall<'_, G> {
    pub(crate) const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    pub(crate) const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact approved grant retained by this call.
    pub(crate) const fn grant(&self) -> RunAccessGrant {
        self.grant
    }

    /// Reauthorizes one additional grant for the same exact root, tenant, and principal.
    pub(crate) async fn authorize_same_run_grant(
        &self,
        grant: RunAccessGrant,
    ) -> Result<(), PublicError> {
        let target = AccessTarget::RunTarget {
            store_scope_id: self.store_scope_id.clone(),
            run_id: self.run_id.clone(),
        };
        let authorized = self
            .policy
            .authorize(&self.credential, grant, &target)
            .await?;
        if authorized.tenant_scope_id() != &self.tenant_scope_id
            || authorized.authenticated_principal_id() != &self.authenticated_principal_id
        {
            return Err(PublicError::grant_denied());
        }
        Ok(())
    }
}

impl AuthorizedRunCall<'_, run_grant::Export> {
    /// Reauthorizes one required export dependency without copying the caller credential.
    ///
    /// Missing, denied, wrong-tenant, wrong-principal, and otherwise inaccessible
    /// dependencies collapse to the same redacted `SourceRunExportDenied` contract.
    /// Dependency identities never appear in the public error.
    pub(crate) async fn authorize_required_dependency(
        &self,
        run_id: RunId,
    ) -> Result<(), PublicError> {
        debug_assert_eq!(self.grant, RunAccessGrant::Export);
        let target = AccessTarget::RunTarget {
            store_scope_id: self.store_scope_id.clone(),
            run_id,
        };
        let authorized = self
            .policy
            .authorize(&self.credential, RunAccessGrant::Export, &target)
            .await
            .map_err(|error| match error {
                crate::AccessPolicyError::AuthenticationRequired => {
                    PublicError::authentication_required()
                }
                crate::AccessPolicyError::GrantDenied => PublicError::source_run_export_denied(),
            })?;
        if authorized.tenant_scope_id() != &self.tenant_scope_id
            || authorized.authenticated_principal_id() != &self.authenticated_principal_id
        {
            return Err(PublicError::source_run_export_denied());
        }
        Ok(())
    }
}

pub(crate) struct AuthorizedAdmissionCall {
    tenant_scope_id: TenantScopeId,
    authenticated_principal_id: StableId,
    entry_point_operation_id: StableId,
    configured_target: StableId,
    invocation_identity: InvocationIdentity,
}

impl AuthorizedAdmissionCall {
    pub(crate) const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    pub(crate) const fn authenticated_principal_id(&self) -> &StableId {
        &self.authenticated_principal_id
    }

    pub(crate) const fn entry_point_operation_id(&self) -> &StableId {
        &self.entry_point_operation_id
    }

    pub(crate) const fn configured_target(&self) -> &StableId {
        &self.configured_target
    }

    pub(crate) const fn invocation_identity(&self) -> &InvocationIdentity {
        &self.invocation_identity
    }
}

#[async_trait]
pub(crate) trait ApplicationBackend: Send + Sync {
    async fn check_ready(&self) -> Result<(), PublicError>;

    async fn admit_run(
        &self,
        call: &AuthorizedAdmissionCall,
        entry_point: EntryPointContract,
        request: AdmitRunRequest,
    ) -> Result<AdmitRunResponse, PublicError>;

    async fn drive_once(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::Drive>,
    ) -> Result<DriveResponse, PublicError>;

    async fn read_public_run(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::ReadPublic>,
    ) -> Result<PublicRunView, PublicError>;

    async fn replay_run(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::Replay>,
        request: ReplayRequest,
    ) -> Result<ReplayResponse, PublicError>;

    async fn read_transition_trace(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::InspectTrace>,
        page: PageRequest,
    ) -> Result<TransitionTracePage, PublicError>;

    async fn read_access_audit(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::InspectAudit>,
        page: PageRequest,
    ) -> Result<AccessAuditPage, PublicError>;

    async fn export_run(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::Export>,
        request: ExportRequest,
    ) -> Result<ExportedRun, PublicError>;
}

/// Value-only behavior available to application transport tests.
///
/// This fixture does not expose the private backend trait, a store, or any authority issuer.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub enum TestApplicationMode {
    /// Every run operation returns one distinctive internal sentinel error.
    Sentinel,
    /// Every run operation returns the tenant-indistinguishable not-found error.
    RunNotFound,
    /// Replay succeeds with the supplied reviewed response; every other run operation is a
    /// sentinel failure.
    Replay(ReplayResponse),
}

/// Builds an application fixture without exposing backend or store authority.
#[cfg(any(test, feature = "test-support"))]
pub fn application_for_test(
    policy: Arc<dyn RunAccessPolicy>,
    entry_points: Vec<EntryPointContract>,
    mode: TestApplicationMode,
) -> Application {
    let store_scope_id = StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
        .expect("the fixed test store scope is valid");
    Application::new(
        store_scope_id,
        policy,
        entry_points,
        TestApplicationBackend { mode, export: None },
    )
}

/// Builds a one-use streaming-export fixture without exposing the private backend trait.
#[cfg(any(test, feature = "test-support"))]
pub fn application_with_export_for_test(
    policy: Arc<dyn RunAccessPolicy>,
    content_ref: mfm_ids::ContentRef,
    reader: crate::ExportAsyncReader,
) -> Application {
    let store_scope_id = StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
        .expect("the fixed test store scope is valid");
    Application::new(
        store_scope_id,
        policy,
        Vec::new(),
        TestApplicationBackend {
            mode: TestApplicationMode::Sentinel,
            export: Some(std::sync::Mutex::new(Some(TestExport {
                content_ref,
                reader,
            }))),
        },
    )
}

#[cfg(any(test, feature = "test-support"))]
struct TestApplicationBackend {
    mode: TestApplicationMode,
    export: Option<std::sync::Mutex<Option<TestExport>>>,
}

#[cfg(any(test, feature = "test-support"))]
struct TestExport {
    content_ref: mfm_ids::ContentRef,
    reader: crate::ExportAsyncReader,
}

#[cfg(any(test, feature = "test-support"))]
impl TestApplicationBackend {
    fn failure(&self) -> PublicError {
        match &self.mode {
            TestApplicationMode::RunNotFound => PublicError::run_not_found(),
            TestApplicationMode::Sentinel | TestApplicationMode::Replay(_) => {
                PublicError::internal(
                    "TestApplicationBackendReached",
                    "The test application backend was reached",
                )
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait]
impl ApplicationBackend for TestApplicationBackend {
    async fn check_ready(&self) -> Result<(), PublicError> {
        Ok(())
    }

    async fn admit_run(
        &self,
        _call: &AuthorizedAdmissionCall,
        _entry_point: EntryPointContract,
        _request: AdmitRunRequest,
    ) -> Result<AdmitRunResponse, PublicError> {
        Err(self.failure())
    }

    async fn drive_once(
        &self,
        _call: &AuthorizedRunCall<'_, run_grant::Drive>,
    ) -> Result<DriveResponse, PublicError> {
        Err(self.failure())
    }

    async fn read_public_run(
        &self,
        _call: &AuthorizedRunCall<'_, run_grant::ReadPublic>,
    ) -> Result<PublicRunView, PublicError> {
        Err(self.failure())
    }

    async fn replay_run(
        &self,
        _call: &AuthorizedRunCall<'_, run_grant::Replay>,
        _request: ReplayRequest,
    ) -> Result<ReplayResponse, PublicError> {
        match &self.mode {
            TestApplicationMode::Replay(response) => Ok(response.clone()),
            TestApplicationMode::Sentinel | TestApplicationMode::RunNotFound => Err(self.failure()),
        }
    }

    async fn read_transition_trace(
        &self,
        _call: &AuthorizedRunCall<'_, run_grant::InspectTrace>,
        _page: PageRequest,
    ) -> Result<TransitionTracePage, PublicError> {
        Err(self.failure())
    }

    async fn read_access_audit(
        &self,
        _call: &AuthorizedRunCall<'_, run_grant::InspectAudit>,
        _page: PageRequest,
    ) -> Result<AccessAuditPage, PublicError> {
        Err(self.failure())
    }

    async fn export_run(
        &self,
        _call: &AuthorizedRunCall<'_, run_grant::Export>,
        _request: ExportRequest,
    ) -> Result<ExportedRun, PublicError> {
        let Some(export) = &self.export else {
            return Err(self.failure());
        };
        let mut export = export.lock().map_err(|_| {
            PublicError::internal(
                "TestApplicationBackendReached",
                "The test application backend was reached",
            )
        })?;
        let export = export.take().ok_or_else(|| self.failure())?;
        Ok(ExportedRun::for_test(export.content_ref, export.reader))
    }
}

/// Connects a production application using opaque exact-target sessions and wallet authorities.
///
/// Production exact reproduction is deliberately unavailable in this cutover. `reproduce`
/// returns the frozen `unavailable` result and never falls back to live runtime capabilities.
/// Deployment infrastructure issues the session bundle; ordinary assembly never receives a pool,
/// URL, connection option, or raw fence.
pub async fn connect_production_application(
    sessions: mfm_storage_postgres::ApplicationTargetSessions,
    policy: Arc<dyn RunAccessPolicy>,
    wallet: EvmWalletDeployment,
) -> Result<Application, PublicError> {
    crate::production::connect(sessions, policy, wallet).await
}

fn wallet_deployment_invalid() -> PublicError {
    PublicError::backend(
        ErrorClass::ServiceUnavailable,
        "EvmWalletDeploymentInvalid",
        "The qualified EVM wallet deployment is invalid",
    )
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll};

    use super::{
        application_for_test, run_grant, AuthorizedRunCall, EvmWalletDeployment,
        EvmWalletDeploymentAssemblyInput, EvmWalletDeploymentReleaseMaterial, TestApplicationMode,
    };
    use crate::{
        AccessPolicyError, AccessTarget, AuthorizedTenant, ExportStreamInput, ReplayRequest,
        RunAccessGrant, RunAccessPolicy, SecretCredential,
    };
    use async_trait::async_trait;
    use mfm_canonical::RecoverabilityContract;
    use mfm_ids::{
        ContentRef, EntryPointId, InvocationIdentity, RunId, SchemaId, StableId, StoreScopeId,
        TenantScopeId,
    };
    use mfm_spec::{CanonicalJsonValue, EntryPointContract, PlanningProfile};
    use mfm_values::MfmValue as _;
    use tokio::io::{AsyncRead, ReadBuf};

    #[test]
    fn wallet_deployment_assembly_authorities_are_affine_and_nonserializable() {
        static_assertions::assert_not_impl_any!(EvmWalletDeployment: Clone, serde::Serialize);
        static_assertions::assert_not_impl_any!(EvmWalletDeploymentAssemblyInput: Clone, serde::Serialize);
        static_assertions::assert_not_impl_any!(EvmWalletDeploymentReleaseMaterial: Clone, serde::Serialize);
    }

    struct FixedPolicy {
        expected_grant: RunAccessGrant,
        result: Result<AuthorizedTenant, AccessPolicyError>,
    }

    #[async_trait]
    impl RunAccessPolicy for FixedPolicy {
        async fn authorize(
            &self,
            credential: &SecretCredential,
            grant: RunAccessGrant,
            _target: &AccessTarget,
        ) -> Result<AuthorizedTenant, AccessPolicyError> {
            assert_eq!(credential.expose_to_policy(), b"opaque");
            assert_eq!(grant, self.expected_grant);
            self.result.clone()
        }
    }

    struct ReplayPolicy {
        replay: Result<AuthorizedTenant, AccessPolicyError>,
        export: Result<AuthorizedTenant, AccessPolicyError>,
        calls: AtomicUsize,
    }

    struct ExactAdmissionTargetPolicy {
        expected_target: StableId,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl RunAccessPolicy for ExactAdmissionTargetPolicy {
        async fn authorize(
            &self,
            credential: &SecretCredential,
            grant: RunAccessGrant,
            target: &AccessTarget,
        ) -> Result<AuthorizedTenant, AccessPolicyError> {
            assert_eq!(credential.expose_to_policy(), b"opaque");
            assert_eq!(grant, RunAccessGrant::Admit);
            self.calls.fetch_add(1, Ordering::SeqCst);
            match target {
                AccessTarget::AdmitTarget {
                    entry_point_operation_id,
                    configured_target,
                    ..
                } if entry_point_operation_id.as_str()
                    == mfm_evm::EVM_SUBMIT_TRANSACTION_OPERATION_ID
                    && configured_target == &self.expected_target =>
                {
                    Ok(AuthorizedTenant::new(tenant('1'), principal('1')))
                }
                AccessTarget::AdmitTarget { .. } => Err(AccessPolicyError::GrantDenied),
                AccessTarget::RunTarget { .. } => panic!("unexpected run target"),
            }
        }
    }

    #[async_trait]
    impl RunAccessPolicy for ReplayPolicy {
        async fn authorize(
            &self,
            _credential: &SecretCredential,
            grant: RunAccessGrant,
            _target: &AccessTarget,
        ) -> Result<AuthorizedTenant, AccessPolicyError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match grant {
                RunAccessGrant::Replay => self.replay.clone(),
                RunAccessGrant::Export => self.export.clone(),
                _ => panic!("unexpected replay policy grant"),
            }
        }
    }

    struct PollProbe {
        polls: Arc<AtomicUsize>,
    }

    impl AsyncRead for PollProbe {
        fn poll_read(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn export_dependencies_have_their_own_redacted_denial_contract() {
        let root_tenant = tenant('1');
        let run_id = run_id();
        let store_scope_id = store_scope();

        let granted = FixedPolicy {
            expected_grant: RunAccessGrant::Export,
            result: Ok(AuthorizedTenant::new(root_tenant.clone(), principal('1'))),
        };
        let call = authorized_export_call(
            &granted,
            &store_scope_id,
            root_tenant.clone(),
            run_id.clone(),
        );
        call.authorize_required_dependency(run_id.clone())
            .await
            .expect("authorized source export");

        let denied = FixedPolicy {
            expected_grant: RunAccessGrant::Export,
            result: Err(AccessPolicyError::GrantDenied),
        };
        let call = authorized_export_call(
            &denied,
            &store_scope_id,
            root_tenant.clone(),
            run_id.clone(),
        );
        let error = call
            .authorize_required_dependency(run_id.clone())
            .await
            .expect_err("denied source export");
        assert_eq!(error.code(), "SourceRunExportDenied");

        let other_tenant = FixedPolicy {
            expected_grant: RunAccessGrant::Export,
            result: Ok(AuthorizedTenant::new(tenant('2'), principal('1'))),
        };
        let call = authorized_export_call(
            &other_tenant,
            &store_scope_id,
            root_tenant.clone(),
            run_id.clone(),
        );
        let error = call
            .authorize_required_dependency(run_id.clone())
            .await
            .expect_err("cross-tenant source export");
        assert_eq!(error.code(), "SourceRunExportDenied");

        let other_principal = FixedPolicy {
            expected_grant: RunAccessGrant::Export,
            result: Ok(AuthorizedTenant::new(root_tenant.clone(), principal('2'))),
        };
        let call = authorized_export_call(
            &other_principal,
            &store_scope_id,
            root_tenant,
            run_id.clone(),
        );
        let error = call
            .authorize_required_dependency(run_id)
            .await
            .expect_err("cross-principal source export");
        assert_eq!(error.code(), "SourceRunExportDenied");
    }

    #[tokio::test]
    async fn non_verify_replay_requires_a_separate_same_identity_export_grant() {
        let root_tenant = tenant('1');
        let run_id = run_id();
        let store_scope_id = store_scope();

        let granted = FixedPolicy {
            expected_grant: RunAccessGrant::Export,
            result: Ok(AuthorizedTenant::new(root_tenant.clone(), principal('1'))),
        };
        let call = authorized_call(
            &granted,
            &store_scope_id,
            root_tenant.clone(),
            run_id.clone(),
            RunAccessGrant::Replay,
        );
        call.authorize_same_run_grant(RunAccessGrant::Export)
            .await
            .expect("same-run semantic export");

        for result in [
            Err(AccessPolicyError::GrantDenied),
            Ok(AuthorizedTenant::new(tenant('2'), principal('1'))),
            Ok(AuthorizedTenant::new(root_tenant.clone(), principal('2'))),
        ] {
            let policy = FixedPolicy {
                expected_grant: RunAccessGrant::Export,
                result,
            };
            let call = authorized_call(
                &policy,
                &store_scope_id,
                root_tenant.clone(),
                run_id.clone(),
                RunAccessGrant::Replay,
            );
            let error = call
                .authorize_same_run_grant(RunAccessGrant::Export)
                .await
                .expect_err("separate export grant");
            assert_eq!(error.code(), "GrantDenied");
        }

        let unauthenticated = FixedPolicy {
            expected_grant: RunAccessGrant::Export,
            result: Err(AccessPolicyError::AuthenticationRequired),
        };
        let call = authorized_call(
            &unauthenticated,
            &store_scope_id,
            root_tenant,
            run_id,
            RunAccessGrant::Replay,
        );
        let error = call
            .authorize_same_run_grant(RunAccessGrant::Export)
            .await
            .expect_err("revoked replay credential");
        assert_eq!(error.code(), "AuthenticationRequired");
    }

    #[tokio::test]
    async fn replay_and_export_denials_leave_the_input_unpolled() {
        let tenant = AuthorizedTenant::new(tenant('1'), principal('1'));
        for (replay, export, expected_calls) in [
            (Err(AccessPolicyError::GrantDenied), Ok(tenant.clone()), 1),
            (Ok(tenant.clone()), Err(AccessPolicyError::GrantDenied), 2),
        ] {
            let policy = Arc::new(ReplayPolicy {
                replay,
                export,
                calls: AtomicUsize::new(0),
            });
            let application =
                application_for_test(policy.clone(), Vec::new(), TestApplicationMode::Sentinel);
            let polls = Arc::new(AtomicUsize::new(0));
            let result = application
                .replay_run(
                    SecretCredential::new(b"opaque".to_vec()).expect("credential"),
                    run_id(),
                    ReplayRequest::Reproduce(stream_input(Arc::clone(&polls))),
                )
                .await;
            assert!(result.is_err());
            assert_eq!(policy.calls.load(Ordering::SeqCst), expected_calls);
            assert_eq!(polls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn backend_history_failure_after_both_grants_leaves_input_unpolled() {
        let tenant = AuthorizedTenant::new(tenant('1'), principal('1'));
        let policy = Arc::new(ReplayPolicy {
            replay: Ok(tenant.clone()),
            export: Ok(tenant),
            calls: AtomicUsize::new(0),
        });
        let application =
            application_for_test(policy.clone(), Vec::new(), TestApplicationMode::Sentinel);
        let polls = Arc::new(AtomicUsize::new(0));
        let error = application
            .replay_run(
                SecretCredential::new(b"opaque".to_vec()).expect("credential"),
                run_id(),
                ReplayRequest::Reproduce(stream_input(Arc::clone(&polls))),
            )
            .await
            .expect_err("backend sentinel");
        assert_eq!(error.code(), "TestApplicationBackendReached");
        assert_eq!(policy.calls.load(Ordering::SeqCst), 2);
        assert_eq!(polls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn admission_authorization_rejects_configured_target_substitution() {
        let expected_target = StableId::new("mfm.evm.test.expected-target").expect("target");
        let policy = Arc::new(ExactAdmissionTargetPolicy {
            expected_target: expected_target.clone(),
            calls: AtomicUsize::new(0),
        });
        let application = application_for_test(
            policy.clone(),
            vec![evm_entry_point()],
            TestApplicationMode::Sentinel,
        );

        let admitted = application
            .admit_run(
                SecretCredential::new(b"opaque".to_vec()).expect("credential"),
                evm_admission_request(expected_target.as_str(), '1'),
            )
            .await
            .expect_err("test backend sentinel");
        assert_eq!(admitted.code(), "TestApplicationBackendReached");

        let substituted = application
            .admit_run(
                SecretCredential::new(b"opaque".to_vec()).expect("credential"),
                evm_admission_request("mfm.evm.test.substituted-target", '2'),
            )
            .await
            .expect_err("substituted target must be denied");
        assert_eq!(substituted.code(), "GrantDenied");
        assert_eq!(policy.calls.load(Ordering::SeqCst), 2);
    }

    fn stream_input(polls: Arc<AtomicUsize>) -> ExportStreamInput {
        let contract = RecoverabilityContract::embedded().expect("recoverability contract");
        let content_ref = ContentRef::new(
            contract
                .schema_id("mfm.portable-run-export-stream.v1")
                .expect("stream schema")
                .clone(),
            contract.raw_content_digest(b"stream"),
        )
        .expect("content ref");
        ExportStreamInput::from_reader(content_ref, Box::pin(PollProbe { polls }))
            .expect("stream input")
    }

    fn evm_admission_request(target: &str, invocation_digit: char) -> crate::AdmitRunRequest {
        let selector = mfm_evm::EvmSubmitTransactionSelector::new(
            mfm_evm::EvmTransactionTarget::new(target).expect("transaction target"),
            mfm_evm::EvmCallerSubmissionToken::new("caller-token").expect("caller token"),
        );
        crate::AdmitRunRequest::new(
            EntryPointId::new(mfm_evm::EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID).expect("entry point"),
            InvocationIdentity::new(format!(
                "00000000-0000-4000-8000-00000000000{invocation_digit}"
            ))
            .expect("invocation"),
            CanonicalJsonValue::new(serde_json::to_value(selector).expect("selector JSON"))
                .expect("canonical selector"),
        )
        .expect("admission request")
    }

    fn evm_entry_point() -> EntryPointContract {
        let planning_profile = PlanningProfile::from_canonical_json(
            br#"{"canonical_profile_parameters":{},"framework_policy_refs":[],"planner_contract_ref":{"content_digest":"content:sha256-v1:1111111111111111111111111111111111111111111111111111111111111111","schema_id":"schema:mfm.test.component:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"},"planner_implementation_ref":{"content_digest":"content:sha256-v1:2222222222222222222222222222222222222222222222222222222222222222","schema_id":"schema:mfm.test.component:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"},"version":"mfm.planning-profile.v1"}"#,
        )
        .expect("planning profile");
        EntryPointContract::new(
            EntryPointId::new(mfm_evm::EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID)
                .expect("entry point"),
            StableId::new(mfm_evm::EVM_SUBMIT_TRANSACTION_OPERATION_ID).expect("operation"),
            planning_profile,
            mfm_evm::EvmSubmitTransactionSelector::schema_id().expect("selector schema"),
            SchemaId::parse("schema:mfm.test.output:1:sha256-jcs-v1:3333333333333333333333333333333333333333333333333333333333333333")
                .expect("output schema"),
        )
        .expect("entry point contract")
    }

    fn authorized_call<'policy>(
        policy: &'policy dyn RunAccessPolicy,
        store_scope_id: &'policy StoreScopeId,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
        grant: RunAccessGrant,
    ) -> AuthorizedRunCall<'policy, run_grant::Replay> {
        AuthorizedRunCall {
            credential: SecretCredential::new(b"opaque".to_vec()).expect("credential"),
            policy,
            store_scope_id,
            tenant_scope_id,
            authenticated_principal_id: principal('1'),
            run_id,
            grant,
            _marker: std::marker::PhantomData,
        }
    }

    fn authorized_export_call<'policy>(
        policy: &'policy dyn RunAccessPolicy,
        store_scope_id: &'policy StoreScopeId,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
    ) -> AuthorizedRunCall<'policy, run_grant::Export> {
        AuthorizedRunCall {
            credential: SecretCredential::new(b"opaque".to_vec()).expect("credential"),
            policy,
            store_scope_id,
            tenant_scope_id,
            authenticated_principal_id: principal('1'),
            run_id,
            grant: RunAccessGrant::Export,
            _marker: std::marker::PhantomData,
        }
    }

    fn store_scope() -> StoreScopeId {
        StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "0".repeat(32)))
            .expect("store scope")
    }

    fn tenant(digit: char) -> TenantScopeId {
        TenantScopeId::new(format!(
            "{}{}",
            TenantScopeId::PREFIX,
            digit.to_string().repeat(32)
        ))
        .expect("tenant scope")
    }

    fn principal(digit: char) -> StableId {
        StableId::new(format!("mfm.test/principal-{digit}")).expect("principal")
    }

    fn run_id() -> RunId {
        RunId::parse(
            "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("run id")
    }
}
