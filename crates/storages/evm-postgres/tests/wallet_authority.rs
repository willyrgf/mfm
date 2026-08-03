//! Managed-PostgreSQL qualification for the wallet activation and nonce authorities.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alloy_primitives::{Address, B256, U256};
use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{EffectAdapterCompletion, ReadAdapterCompletion};
use mfm_evm::{
    canonical_wallet_reference, derive_authenticated_intent_issuer_id,
    derive_evm_candidate_operation_key, derive_evm_chain_lineage_id,
    derive_evm_nonce_completion_key, derive_evm_nonce_reservation_key,
    derive_exact_candidate_activation_permit, derive_submission_intent_id,
    derive_wallet_nonce_domain, evm_wallet_assurance_policy_ref, evm_wallet_nonce_policy_ref,
    ActivateCandidateResponse, ActivateEvmCandidateRequest, ActiveWalletCandidate,
    AttestedWalletCandidate, CandidateActivationPermit, CanonicalTerminalOutcome,
    ChainInstanceDeclaration, ChainInstanceRegistryAttestation, CompleteEvmNonceRequest,
    CompleteWalletNonceResponse, CompletedWalletNonce, EvmCandidateFamily,
    EvmFinalizedHeadObservation, EvmInclusionBlockObservation, EvmReceiptLookupObservation,
    EvmRoutingCatalogDescriptor, EvmRoutingGenerationDescriptor, EvmSubmissionFailure,
    EvmTransactionIntent, EvmTransactionLookupObservation, EvmTransactionTarget,
    EvmWalletFeeCandidate, EvmWalletReference, EvmWalletTransactionAction,
    EvmWalletTransactionTemplate, ExclusiveCurrentControl, ExecutionDisposition,
    ObservedPendingNonceFloor, PriorEffectDisposition, PriorResourceDisposition,
    QualifiedPendingNonceFloor, ReadEvmWalletNonceStatusRequest, ReplayExclusionDisposition,
    ReserveEvmNonceRequest, ReserveWalletNonceResponse, ReservedWalletNonce, TerminalWitnesses,
    UnsignedWalletCandidate, WalletNonceAuthority, WalletNonceDomainActivationAttestation,
    WalletNonceDomainActivationRecord, WalletNonceStatus, WalletNonceStoreIncarnation,
    WalletNonceStoreLineageHead, WalletNonceStoreSuccessor,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId, TenantScopeId};
use mfm_journal::structured::{canonical_json, HistoryObject, LexicalValueRef, TypedValueRef};
use mfm_storage_evm_postgres::{
    open_activation_registry_admin, open_activation_registry_public, open_wallet_nonce_authority,
    ActivationIssuanceFaultPoint, DeploymentAssemblyBinding, DeploymentAssemblyRouteProof,
    PendingDeploymentAssembly, PostgresEvmWalletError, PostgresEvmWalletSchema,
    PostgresWalletNonceAuthority, WalletAuthorityProviderClient, WalletAuthorityProviderTrust,
};
use mfm_wallet_authority_provider_test_support::{
    postgres_proxy::{CommitFault, PostgresCommitFaultProxy},
    AllowedPromotion, ProviderCheckpointAuthority, ProviderDeploymentAssemblyPolicy,
    ProviderProcess, ProviderProcessConfig, ProviderPromotionCrashPoint,
    ProviderRpcInventoryTarget, ProviderStartupState, SupersessionConfig,
};
use ring::hmac;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, ConnectOptions, Connection, Executor, PgConnection, PgPool};

static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);
static LOGIN_PRINCIPALS_CONFIGURED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
static MANAGED_WALLET_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const ACTIVATION_ADMIN_DATABASE_URL_ENV: &str = "MFM_EVM_WALLET_ACTIVATION_ADMIN_DATABASE_URL";
const ACTIVATION_PUBLIC_DATABASE_URL_ENV: &str = "MFM_EVM_WALLET_ACTIVATION_PUBLIC_DATABASE_URL";
const NONCE_APPLICATION_DATABASE_URL_ENV: &str = "MFM_EVM_WALLET_NONCE_APPLICATION_DATABASE_URL";
const ACTIVATION_ADMIN_ROLE: &str = "mfm_evm_wallet_activation_admin";
const DEPLOYMENT_RPC_TARGET_IDENTITY_BASE: u8 = 0x51;
const DEPLOYMENT_RPC_PROOF_KEY_BASE: u8 = 0x72;
const MANAGED_WALLET_TEST_STACK_BYTES: usize = 32 * 1024 * 1024;

#[test]
fn real_sql_authority_preserves_activation_nonce_and_role_boundaries() {
    run_managed_wallet_test(
        real_sql_authority_preserves_activation_nonce_and_role_boundaries_inner,
    );
}

#[test]
fn deployment_assembly_protocol_rejects_every_hostile_affine_cutover() {
    run_managed_wallet_test(
        deployment_assembly_protocol_rejects_every_hostile_affine_cutover_inner,
    );
}

fn run_managed_wallet_test<F, Fut>(test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name("managed-wallet-parity".to_owned())
        .stack_size(MANAGED_WALLET_TEST_STACK_BYTES)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build managed wallet test runtime")
                .block_on(async move {
                    let _test_guard = MANAGED_WALLET_TEST_LOCK.lock().await;
                    Box::pin(test()).await;
                });
        })
        .expect("spawn managed wallet test thread")
        .join()
        .expect("join managed wallet test thread");
}

#[derive(Clone)]
struct DeploymentProtocolPolicy {
    signer_generation_ref: ContentRef,
    signer_fence_ref: ContentRef,
    direct_sign_exclusion_ref: ContentRef,
    semantic_contract_refs: Vec<ContentRef>,
    release_history_digests: Vec<ContentDigest>,
}

impl DeploymentProtocolPolicy {
    fn new() -> Self {
        Self {
            signer_generation_ref: protocol_content_ref("deployment-signer-generation"),
            signer_fence_ref: protocol_content_ref("deployment-signer-fence"),
            direct_sign_exclusion_ref: protocol_content_ref("deployment-direct-sign-exclusion"),
            semantic_contract_refs: (0_u8..6)
                .map(|ordinal| protocol_content_ref(&format!("deployment-semantic-{ordinal}")))
                .collect(),
            release_history_digests: (0_u8..6)
                .map(|ordinal| {
                    ContentDigest::from_digest(
                        DigestAlgorithm::Sha256V1,
                        sha256_digest_bytes(&[0xa0 + ordinal]),
                    )
                })
                .collect(),
        }
    }

    fn provider_policy(&self, fixture: &Fixture) -> ProviderDeploymentAssemblyPolicy {
        ProviderDeploymentAssemblyPolicy::new(
            fixture.semantic_signer_id.clone(),
            self.signer_generation_ref.clone(),
            self.signer_fence_ref.clone(),
            self.direct_sign_exclusion_ref.clone(),
            self.semantic_contract_refs.clone(),
            self.release_history_digests.clone(),
        )
        .expect("deployment protocol policy")
    }

    fn binding(
        &self,
        fixture: &Fixture,
        activation: WalletNonceDomainActivationAttestation,
    ) -> DeploymentAssemblyBinding {
        DeploymentAssemblyBinding::new(
            activation,
            fixture.incarnation.clone(),
            fixture.current_public_head.clone(),
            fixture.provider_fence_head_ref.clone(),
            fixture.semantic_signer_id.clone(),
            self.signer_generation_ref.clone(),
            self.signer_fence_ref.clone(),
            self.direct_sign_exclusion_ref.clone(),
            self.semantic_contract_refs.clone(),
            self.release_history_digests.clone(),
        )
        .expect("deployment protocol binding")
    }
}

async fn deployment_assembly_protocol_rejects_every_hostile_affine_cutover_inner() {
    let base_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL is required for parity tests");
    let fixture = Fixture::new();
    let policy = DeploymentProtocolPolicy::new();
    let schema_a = format!("{}_deployment_a", unique_schema());
    let schema_b = format!("{}_deployment_b", unique_schema());
    let admin_options = PgConnectOptions::from_str(&base_url).expect("parse DATABASE_URL");
    let mut admin = PgConnection::connect_with(&admin_options)
        .await
        .expect("connect PostgreSQL administrator");
    for schema in [&schema_a, &schema_b] {
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&mut admin)
            .await
            .expect("create deployment protocol schema");
        PostgresEvmWalletSchema::migrate(&scoped_database_url(&base_url, schema))
            .await
            .expect("migrate deployment protocol schema");
    }
    let role_urls_a = WalletRoleDatabaseUrls::for_schema(&schema_a);
    let role_urls_b = WalletRoleDatabaseUrls::for_schema(&schema_b);
    configure_wallet_login_principals(&mut admin, &role_urls_a).await;

    let checkpoint_a = ProviderCheckpointAuthority::start(
        &database_url_with_active_role(&role_urls_a.activation_admin, ACTIVATION_ADMIN_ROLE),
        &database_url_with_active_role(
            &role_urls_a.nonce_application,
            "mfm_evm_wallet_nonce_application",
        ),
        &schema_a,
        fixture.provider_fence_lineage_ref.clone(),
        fixture.incarnation.clone(),
        fixture.current_public_head.clone(),
        fixture.provider_fence_head_ref.clone(),
    )
    .await
    .expect("start deployment protocol checkpoint A");
    let base_config_a = deployment_protocol_provider_config(
        &checkpoint_a,
        &role_urls_a,
        &schema_a,
        fixture.provider_id.clone(),
        &fixture,
        &policy,
        vec![fixture.routing_catalog.clone()],
    );
    let mut provider_a = spawn_deployment_protocol_provider(&base_config_a);
    let client_a = deployment_protocol_client(
        &provider_a,
        &fixture,
        fixture.provider_id.clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
    )
    .await;
    let activation_a =
        issue_deployment_protocol_activation(&role_urls_a, &schema_a, &fixture, client_a.clone())
            .await;
    assert_wallet_nonce_schema_empty(&role_urls_a.nonce_application, &schema_a).await;

    let stale_begin_catalog = client_a
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify H catalog for stale Begin");
    let stale_finish_catalog = client_a
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify H catalog for stale Finish");
    let stale_pending = stale_finish_catalog
        .begin_deployment_assembly(policy.binding(&fixture, activation_a.clone()))
        .await
        .expect("begin H deployment before H+1");
    let (stale_exchange_ref, stale_proofs) = deployment_protocol_proofs(&stale_pending);
    assert_eq!(provider_a.active_leases().expect("observe H lease"), 1);
    drop(provider_a);

    let refreshed_head = authority_reference("chain-registry-head-deployment-refreshed");
    let refreshed_catalog = EvmRoutingCatalogDescriptor::new(
        refreshed_head.clone(),
        fixture.routing_catalog.chain_instances().to_vec(),
        fixture.routing_catalog.generations().to_vec(),
    )
    .expect("append-only H+1 routing catalog");
    let refreshed_config_a = base_config_a
        .clone()
        .with_routing_catalog_history(
            refreshed_head.clone(),
            vec![fixture.routing_catalog.clone(), refreshed_catalog.clone()],
        )
        .expect("configure H+1 provider");
    provider_a = spawn_deployment_protocol_provider(&refreshed_config_a);
    let client_a_h1 = deployment_protocol_client(
        &provider_a,
        &fixture,
        fixture.provider_id.clone(),
        refreshed_head,
    )
    .await;
    assert_protocol_rejection(
        client_a_h1
            .qualify_routing_catalog(&fixture.routing_catalog)
            .await,
        &[],
    );
    assert_protocol_rejection(
        stale_begin_catalog
            .begin_deployment_assembly(policy.binding(&fixture, activation_a.clone()))
            .await,
        &[],
    );
    assert_protocol_rejection(
        stale_pending.finish(stale_exchange_ref, stale_proofs).await,
        &[],
    );
    assert_eq!(provider_a.active_leases().expect("observe H+1 leases"), 0);
    assert_eq!(
        provider_a
            .successful_deployment_finishes()
            .expect("observe H+1 finishes"),
        0
    );
    let fresh_h1 = client_a_h1
        .qualify_routing_catalog(&refreshed_catalog)
        .await
        .expect("qualify fresh H+1 catalog");
    let fresh_h1_pending = fresh_h1
        .begin_deployment_assembly(policy.binding(&fixture, activation_a.clone()))
        .await
        .expect("begin fresh H+1 deployment");
    assert_eq!(
        provider_a.active_leases().expect("observe fresh H+1 lease"),
        1
    );
    drop(fresh_h1_pending);
    provider_a.revoke().expect("revoke fresh H+1 provider");
    assert_eq!(
        provider_a.active_leases().expect("observe H+1 revocation"),
        0
    );
    drop(provider_a);

    let duplicate_marker = [0xd1; 32];
    let duplicate_config = base_config_a
        .clone()
        .with_hostile_deployment_assembly_lease_sequence(vec![duplicate_marker, duplicate_marker])
        .expect("configure duplicate deployment marker");
    provider_a = spawn_deployment_protocol_provider(&duplicate_config);
    let duplicate_client = deployment_protocol_client(
        &provider_a,
        &fixture,
        fixture.provider_id.clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
    )
    .await;
    let first_catalog = duplicate_client
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify first duplicate-marker catalog");
    let second_catalog = duplicate_client
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify second duplicate-marker catalog");
    let first_pending = first_catalog
        .begin_deployment_assembly(policy.binding(&fixture, activation_a.clone()))
        .await
        .expect("begin first duplicate-marker deployment");
    assert_eq!(first_pending.assembly_lease(), &duplicate_marker);
    assert_protocol_rejection(
        second_catalog
            .begin_deployment_assembly(policy.binding(&fixture, activation_a.clone()))
            .await,
        &[hex::encode(duplicate_marker)],
    );
    assert_eq!(
        provider_a
            .active_leases()
            .expect("observe unoverwritten lease"),
        1
    );
    let (exchange_ref, proofs) = deployment_protocol_proofs(&first_pending);
    let finished = first_pending
        .finish(exchange_ref, proofs)
        .await
        .expect("finish original duplicate-marker lease once");
    drop(finished);
    assert_eq!(
        provider_a.active_leases().expect("observe consumed lease"),
        0
    );
    assert_eq!(
        provider_a
            .successful_deployment_finishes()
            .expect("observe one finish"),
        1
    );
    drop(provider_a);

    provider_a = spawn_deployment_protocol_provider(&base_config_a);
    let timeout_client = deployment_protocol_client(
        &provider_a,
        &fixture,
        fixture.provider_id.clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
    )
    .await;
    let timeout_pending = timeout_client
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify timeout catalog")
        .begin_deployment_assembly(policy.binding(&fixture, activation_a.clone()))
        .await
        .expect("begin expiring deployment");
    let timeout_lease = hex::encode(timeout_pending.assembly_lease());
    let (timeout_exchange_ref, timeout_proofs) = deployment_protocol_proofs(&timeout_pending);
    tokio::time::sleep(Duration::from_secs(31)).await;
    assert_protocol_rejection(
        timeout_pending
            .finish(timeout_exchange_ref, timeout_proofs)
            .await,
        &[timeout_lease],
    );
    assert_provider_has_no_partial_deployment(&mut provider_a, 0);
    drop(provider_a);

    provider_a = spawn_deployment_protocol_provider(&base_config_a);
    let disconnect_client = deployment_protocol_client(
        &provider_a,
        &fixture,
        fixture.provider_id.clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
    )
    .await;
    let disconnected_pending = disconnect_client
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify disconnected catalog")
        .begin_deployment_assembly(policy.binding(&fixture, activation_a.clone()))
        .await
        .expect("begin disconnected deployment");
    drop(disconnected_pending);
    assert_eq!(
        provider_a
            .active_leases()
            .expect("observe disconnected lease"),
        1
    );
    provider_a.revoke().expect("revoke disconnected deployment");
    assert_provider_has_no_partial_deployment(&mut provider_a, 0);
    drop(provider_a);

    provider_a = spawn_deployment_protocol_provider(&base_config_a);
    let revoked_client = deployment_protocol_client(
        &provider_a,
        &fixture,
        fixture.provider_id.clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
    )
    .await;
    let revoked_pending = revoked_client
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify revocation catalog")
        .begin_deployment_assembly(policy.binding(&fixture, activation_a.clone()))
        .await
        .expect("begin deployment before revocation");
    let revoked_lease = hex::encode(revoked_pending.assembly_lease());
    let (revoked_exchange_ref, revoked_proofs) = deployment_protocol_proofs(&revoked_pending);
    provider_a.revoke().expect("revoke pending deployment");
    assert_protocol_rejection(
        revoked_pending
            .finish(revoked_exchange_ref, revoked_proofs)
            .await,
        &[revoked_lease],
    );
    assert_provider_has_no_partial_deployment(&mut provider_a, 0);
    drop(provider_a);

    provider_a = spawn_deployment_protocol_provider(&base_config_a);
    let crash_client = deployment_protocol_client(
        &provider_a,
        &fixture,
        fixture.provider_id.clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
    )
    .await;
    let crashed_pending = crash_client
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify crash catalog")
        .begin_deployment_assembly(policy.binding(&fixture, activation_a.clone()))
        .await
        .expect("begin deployment before crash");
    let crashed_lease = hex::encode(crashed_pending.assembly_lease());
    let (crashed_exchange_ref, crashed_proofs) = deployment_protocol_proofs(&crashed_pending);
    provider_a.crash().expect("crash deployment provider");
    assert_protocol_rejection(
        crashed_pending
            .finish(crashed_exchange_ref, crashed_proofs)
            .await,
        &[crashed_lease],
    );
    drop(provider_a);
    provider_a = spawn_deployment_protocol_provider(&base_config_a);
    assert_provider_has_no_partial_deployment(&mut provider_a, 0);
    let crash_control = deployment_protocol_client(
        &provider_a,
        &fixture,
        fixture.provider_id.clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
    )
    .await
    .qualify_routing_catalog(&fixture.routing_catalog)
    .await
    .expect("qualify fresh post-crash catalog")
    .begin_deployment_assembly(policy.binding(&fixture, activation_a.clone()))
    .await
    .expect("begin fresh post-crash deployment");
    drop(crash_control);
    provider_a.revoke().expect("revoke post-crash control");
    assert_provider_has_no_partial_deployment(&mut provider_a, 0);
    drop(provider_a);

    let provider_b_id = stable("mfm.evm.test/wallet-authority-provider-b");
    let checkpoint_b = ProviderCheckpointAuthority::start(
        &database_url_with_active_role(&role_urls_b.activation_admin, ACTIVATION_ADMIN_ROLE),
        &database_url_with_active_role(
            &role_urls_b.nonce_application,
            "mfm_evm_wallet_nonce_application",
        ),
        &schema_b,
        fixture.provider_fence_lineage_ref.clone(),
        fixture.incarnation.clone(),
        fixture.current_public_head.clone(),
        fixture.provider_fence_head_ref.clone(),
    )
    .await
    .expect("start deployment protocol checkpoint B");
    let config_b = deployment_protocol_provider_config(
        &checkpoint_b,
        &role_urls_b,
        &schema_b,
        provider_b_id.clone(),
        &fixture,
        &policy,
        vec![fixture.routing_catalog.clone()],
    );
    let mut provider_b = spawn_deployment_protocol_provider(&config_b);
    let client_b = deployment_protocol_client(
        &provider_b,
        &fixture,
        provider_b_id,
        fixture.routing_catalog.chain_registry_head_ref().clone(),
    )
    .await;
    let activation_b =
        issue_deployment_protocol_activation(&role_urls_b, &schema_b, &fixture, client_b.clone())
            .await;
    assert_ne!(
        activation_a.registry_issuance_ref, activation_b.registry_issuance_ref,
        "distinct logical providers must issue distinct activation references"
    );
    provider_a = spawn_deployment_protocol_provider(&base_config_a);
    let mixed_catalog = deployment_protocol_client(
        &provider_a,
        &fixture,
        fixture.provider_id.clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
    )
    .await
    .qualify_routing_catalog(&fixture.routing_catalog)
    .await
    .expect("qualify provider-A mixed catalog");
    assert_protocol_rejection(
        mixed_catalog
            .begin_deployment_assembly(policy.binding(&fixture, activation_b.clone()))
            .await,
        &[],
    );
    assert_provider_has_no_partial_deployment(&mut provider_a, 0);
    let provider_b_control = client_b
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify provider-B control catalog")
        .begin_deployment_assembly(policy.binding(&fixture, activation_b))
        .await
        .expect("begin provider-B control deployment");
    drop(provider_b_control);
    provider_b.revoke().expect("revoke provider-B control");
    assert_provider_has_no_partial_deployment(&mut provider_b, 0);
    drop(provider_a);
    drop(provider_b);
    drop(checkpoint_a);
    drop(checkpoint_b);

    assert_wallet_nonce_schema_empty(&role_urls_a.nonce_application, &schema_a).await;
    assert_wallet_nonce_schema_empty(&role_urls_b.nonce_application, &schema_b).await;
    for schema in [&schema_a, &schema_b] {
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&mut admin)
            .await
            .expect("drop deployment protocol schema");
    }
}

fn deployment_protocol_provider_config(
    checkpoint: &ProviderCheckpointAuthority,
    role_urls: &WalletRoleDatabaseUrls,
    schema: &str,
    provider_id: StableId,
    fixture: &Fixture,
    policy: &DeploymentProtocolPolicy,
    routing_catalog_history: Vec<EvmRoutingCatalogDescriptor>,
) -> ProviderProcessConfig {
    ProviderProcessConfig::new(
        unique_provider_socket_path(),
        checkpoint,
        database_url_with_active_role(&role_urls.activation_admin, ACTIVATION_ADMIN_ROLE),
        database_url_with_active_role(
            &role_urls.nonce_application,
            "mfm_evm_wallet_nonce_application",
        ),
        schema.to_owned(),
        provider_id,
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        routing_catalog_history
            .last()
            .expect("current deployment catalog")
            .chain_registry_head_ref()
            .clone(),
        fixture.registry_lineage_ref.clone(),
        fixture.provider_fence_head_ref.clone(),
        fixture.incarnation.clone(),
        fixture.current_public_head.clone(),
        routing_catalog_history,
        vec![fixture.activation_record.clone()],
        Vec::new(),
        None,
    )
    .expect("configure deployment protocol provider")
    .with_rpc_inventory_targets(deployment_protocol_targets(&fixture.routing_catalog))
    .expect("configure deployment protocol targets")
    .with_deployment_assembly_policies(vec![policy.provider_policy(fixture)])
    .expect("configure deployment protocol policy")
}

fn deployment_protocol_targets(
    catalog: &EvmRoutingCatalogDescriptor,
) -> Vec<ProviderRpcInventoryTarget> {
    catalog
        .generations()
        .iter()
        .enumerate()
        .map(|(ordinal, generation)| {
            ProviderRpcInventoryTarget::new(
                generation
                    .generation_ref()
                    .and_then(|reference| {
                        reference
                            .to_content_ref()
                            .map_err(|_| mfm_evm::WalletAuthorityContractError::Invalid("route"))
                    })
                    .expect("deployment target route"),
                [DEPLOYMENT_RPC_TARGET_IDENTITY_BASE + ordinal as u8; 32],
                [DEPLOYMENT_RPC_PROOF_KEY_BASE + ordinal as u8; 32],
            )
        })
        .collect()
}

fn spawn_deployment_protocol_provider(config: &ProviderProcessConfig) -> ProviderProcess {
    ProviderProcess::spawn(
        env!("CARGO_BIN_EXE_mfm-storage-wallet-authority-provider"),
        config.clone(),
    )
    .expect("start deployment protocol provider")
}

async fn deployment_protocol_client(
    provider: &ProviderProcess,
    fixture: &Fixture,
    provider_id: StableId,
    current_chain_registry_head_ref: EvmWalletReference,
) -> WalletAuthorityProviderClient {
    let trust = WalletAuthorityProviderTrust::new(
        provider_id,
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        current_chain_registry_head_ref,
        provider.public_key_hex(),
    )
    .expect("deployment protocol trust");
    WalletAuthorityProviderClient::connect_unix(provider.endpoint(), trust)
        .await
        .expect("authenticate deployment protocol provider")
}

async fn issue_deployment_protocol_activation(
    role_urls: &WalletRoleDatabaseUrls,
    schema: &str,
    fixture: &Fixture,
    client: WalletAuthorityProviderClient,
) -> WalletNonceDomainActivationAttestation {
    let registry = open_activation_registry_admin(&role_urls.activation_admin, schema, client)
        .await
        .expect("open deployment activation registry");
    registry
        .issue_domain_activation(&fixture.activation_record, &fixture.incarnation)
        .await
        .expect("issue deployment protocol activation")
}

fn deployment_protocol_proofs(
    pending: &PendingDeploymentAssembly,
) -> ([u8; 32], Vec<DeploymentAssemblyRouteProof>) {
    let assembly_lease = hex::encode(pending.assembly_lease());
    let checkpoint = hex::encode(pending.checkpoint());
    let finish_commitment = hex::encode(pending.finish_authorization_commitment());
    let mut exchange = Vec::new();
    let proofs = pending
        .route_challenges()
        .enumerate()
        .map(|(ordinal, challenge)| {
            let ordinal = u16::try_from(ordinal).expect("bounded deployment proof ordinal");
            let target_identity = [DEPLOYMENT_RPC_TARGET_IDENTITY_BASE + ordinal as u8; 32];
            assert_eq!(challenge.target_identity(), &target_identity);
            let route_challenge = hex::encode(challenge.route_challenge());
            let proof_message = serde_json::to_vec(&(
                ordinal,
                challenge.route_generation_ref(),
                hex::encode(target_identity),
                &assembly_lease,
                &checkpoint,
                &route_challenge,
                &finish_commitment,
            ))
            .expect("encode deployment proof message");
            let proof_key = [DEPLOYMENT_RPC_PROOF_KEY_BASE + ordinal as u8; 32];
            let key = hmac::Key::new(hmac::HMAC_SHA256, &proof_key);
            let proof = hmac::sign(&key, &proof_message).as_ref().to_vec();
            append_protocol_frame(&mut exchange, &ordinal.to_be_bytes());
            append_protocol_frame(
                &mut exchange,
                &serde_json::to_vec(challenge.route_generation_ref())
                    .expect("encode deployment route reference"),
            );
            append_protocol_frame(&mut exchange, &target_identity);
            append_protocol_frame(&mut exchange, pending.assembly_lease());
            append_protocol_frame(&mut exchange, pending.checkpoint());
            append_protocol_frame(&mut exchange, challenge.route_challenge());
            append_protocol_frame(&mut exchange, pending.finish_authorization_commitment());
            append_protocol_frame(&mut exchange, &proof);
            DeploymentAssemblyRouteProof::new(
                ordinal,
                challenge.route_generation_ref().clone(),
                proof.into_boxed_slice(),
            )
            .expect("construct deployment protocol proof")
        })
        .collect();
    (*sha256_digest_bytes(&exchange).as_bytes(), proofs)
}

fn append_protocol_frame(output: &mut Vec<u8>, value: &[u8]) {
    let length = u32::try_from(value.len()).expect("bounded deployment protocol frame");
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
}

fn assert_protocol_rejection<T>(
    result: Result<T, PostgresEvmWalletError>,
    private_values: &[String],
) {
    let error = match result {
        Ok(_) => panic!("hostile deployment protocol unexpectedly succeeded"),
        Err(error) => error,
    };
    let rendered = error.to_string();
    for private in private_values {
        assert!(
            !rendered.contains(private),
            "deployment protocol error exposed private material"
        );
    }
    assert!(
        rendered.len() < 256,
        "deployment protocol error must stay bounded"
    );
}

fn assert_provider_has_no_partial_deployment(provider: &mut ProviderProcess, finishes: u64) {
    assert_eq!(
        provider
            .active_leases()
            .expect("observe affine lease closure"),
        0
    );
    assert_eq!(
        provider
            .successful_deployment_finishes()
            .expect("observe deployment Finish count"),
        finishes
    );
}

fn protocol_content_ref(label: &str) -> ContentRef {
    authority_reference(label)
        .to_content_ref()
        .expect("deployment protocol content reference")
}

async fn real_sql_authority_preserves_activation_nonce_and_role_boundaries_inner() {
    let base_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL is required for parity tests");
    let schema = unique_schema();
    let admin_options = PgConnectOptions::from_str(&base_url).expect("parse DATABASE_URL");
    let mut admin_connection = PgConnection::connect_with(&admin_options)
        .await
        .expect("connect PostgreSQL administrator");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&mut admin_connection)
        .await
        .expect("create isolated wallet schema");
    let public_sentinel = format!("{schema}_sentinel");
    sqlx::query(AssertSqlSafe(format!(
        "CREATE TABLE public.{public_sentinel} (value INTEGER NOT NULL)"
    )))
    .execute(&mut admin_connection)
    .await
    .expect("create public-schema sentinel");
    let database_url = scoped_database_url(&base_url, &schema);
    PostgresEvmWalletSchema::migrate(&database_url)
        .await
        .expect("migrate wallet authority");
    let role_urls = WalletRoleDatabaseUrls::for_schema(&schema);
    configure_wallet_login_principals(&mut admin_connection, &role_urls).await;
    prove_hostile_qualification_reopens(
        &mut admin_connection,
        &role_urls.activation_public,
        &schema,
    )
    .await;

    let sibling_schema = format!("{schema}_sibling");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {sibling_schema}")))
        .execute(&mut admin_connection)
        .await
        .expect("create sibling wallet schema");
    let sibling_schema_url = scoped_database_url(&base_url, &sibling_schema);
    PostgresEvmWalletSchema::migrate(&sibling_schema_url)
        .await
        .expect("migrate sibling wallet schema");
    let sibling_schema_role_urls = WalletRoleDatabaseUrls::for_schema(&sibling_schema);

    let sibling_database = format!("{schema}_db");
    sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {sibling_database}")))
        .execute(&mut admin_connection)
        .await
        .expect("create sibling wallet database");
    let sibling_database_base_url = database_url_with_database(&base_url, &sibling_database);
    let sibling_database_options =
        PgConnectOptions::from_str(&sibling_database_base_url).expect("parse sibling database URL");
    let mut sibling_database_connection = PgConnection::connect_with(&sibling_database_options)
        .await
        .expect("connect sibling wallet database");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&mut sibling_database_connection)
        .await
        .expect("create wallet schema in sibling database");
    let sibling_database_url = scoped_database_url(&sibling_database_base_url, &schema);
    PostgresEvmWalletSchema::migrate(&sibling_database_url)
        .await
        .expect("migrate sibling wallet database");
    let sibling_database_role_urls =
        WalletRoleDatabaseUrls::for_database_and_schema(&sibling_database, &schema);

    let fixture = Fixture::new();
    let promotion_successor = fixture.promotion_successor();
    let fault_incarnation = initial_incarnation(
        &fixture,
        "mfm.evm.test/fault-lineage",
        "mfm.evm.test/fault-target",
    );
    let fault_record = activation_record_for_sender(&fixture, 0x31, &fault_incarnation);
    let competing_incarnation_a = initial_incarnation(
        &fixture,
        "mfm.evm.test/race-lineage-a",
        "mfm.evm.test/race-target-a",
    );
    let competing_incarnation_b = initial_incarnation(
        &fixture,
        "mfm.evm.test/race-lineage-b",
        "mfm.evm.test/race-target-b",
    );
    let competing_record_a = activation_record_for_sender(&fixture, 0x32, &competing_incarnation_a);
    let competing_record_b = activation_record_for_sender(&fixture, 0x32, &competing_incarnation_b);
    let sibling_incarnation_a = initial_incarnation(
        &fixture,
        "mfm.evm.test/sibling-race-lineage",
        "mfm.evm.test/sibling-race-target-a",
    );
    let sibling_incarnation_b = initial_incarnation(
        &fixture,
        "mfm.evm.test/sibling-race-lineage",
        "mfm.evm.test/sibling-race-target-b",
    );
    let sibling_record_a = activation_record_for_sender(&fixture, 0x33, &sibling_incarnation_a);
    let sibling_record_b = activation_record_for_sender(&fixture, 0x34, &sibling_incarnation_b);
    assert_ne!(
        sibling_record_a.wallet_nonce_domain,
        sibling_record_b.wallet_nonce_domain
    );
    assert_eq!(
        sibling_record_a.wallet_nonce_store_lineage_id,
        sibling_record_b.wallet_nonce_store_lineage_id
    );
    assert_ne!(
        sibling_record_a.initial_store_incarnation_ref,
        sibling_record_b.initial_store_incarnation_ref
    );
    let checkpoint_authority = ProviderCheckpointAuthority::start(
        &database_url_with_active_role(&role_urls.activation_admin, ACTIVATION_ADMIN_ROLE),
        &database_url_with_active_role(
            &role_urls.nonce_application,
            "mfm_evm_wallet_nonce_application",
        ),
        &schema,
        fixture.provider_fence_lineage_ref.clone(),
        fixture.incarnation.clone(),
        fixture.current_public_head.clone(),
        fixture.provider_fence_head_ref.clone(),
    )
    .await
    .expect("start external wallet checkpoint authority");
    let provider_config = ProviderProcessConfig::new(
        std::env::temp_dir().join(format!("{schema}.sock")),
        &checkpoint_authority,
        database_url_with_active_role(&role_urls.activation_admin, ACTIVATION_ADMIN_ROLE),
        database_url_with_active_role(
            &role_urls.nonce_application,
            "mfm_evm_wallet_nonce_application",
        ),
        schema.clone(),
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        fixture.registry_lineage_ref.clone(),
        fixture.provider_fence_head_ref.clone(),
        fixture.incarnation.clone(),
        fixture.current_public_head.clone(),
        vec![fixture.routing_catalog.clone()],
        vec![
            fixture.activation_record.clone(),
            fixture.secondary_activation_record.clone(),
            fault_record.clone(),
            competing_record_a.clone(),
            competing_record_b.clone(),
            sibling_record_a.clone(),
            sibling_record_b.clone(),
        ],
        vec![AllowedPromotion {
            successor: promotion_successor.clone(),
            next_public_lineage_head: fixture.next_public_head.clone(),
            next_provider_fence_head_ref: fixture.next_provider_fence_head_ref.clone(),
        }],
        Some(SupersessionConfig {
            evidence: fixture.next_lineage_head.clone(),
            public_head: fixture.next_public_head.clone(),
        }),
    )
    .expect("configure separate wallet authority provider")
    .with_allowed_issuance_incarnations(vec![
        fixture.incarnation.clone(),
        fault_incarnation.clone(),
        competing_incarnation_a.clone(),
        competing_incarnation_b.clone(),
        sibling_incarnation_a.clone(),
        sibling_incarnation_b.clone(),
    ])
    .expect("allow exact hostile activation-race incarnations");
    assert_hostile_routing_history_restart_fails_closed(&provider_config, &fixture);
    assert_activation_requires_issued_chain_and_exact_route(
        &role_urls.activation_admin,
        &role_urls.nonce_application,
        &schema,
        &fixture,
        &checkpoint_authority,
    );
    let mut provider = ProviderProcess::spawn(
        env!("CARGO_BIN_EXE_mfm-storage-wallet-authority-provider"),
        provider_config.clone(),
    )
    .expect("start separate wallet authority provider");
    let provider_public_key = provider.public_key_hex().to_owned();
    let untrusted_provider = WalletAuthorityProviderTrust::new(
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        &"00".repeat(32),
    )
    .expect("construct intentionally wrong provider trust anchor");
    assert!(matches!(
        WalletAuthorityProviderClient::connect_unix(provider.endpoint(), untrusted_provider).await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));
    let sibling_registry_trust = WalletAuthorityProviderTrust::new(
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        evm_wallet_assurance_policy_ref().expect("sibling chain-registry lineage"),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        &provider_public_key,
    )
    .expect("construct sibling-registry trust anchor");
    assert!(matches!(
        WalletAuthorityProviderClient::connect_unix(provider.endpoint(), sibling_registry_trust,)
            .await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));
    let rolled_back_head_trust = WalletAuthorityProviderTrust::new(
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        authority_reference("chain-registry-head-rolled-back"),
        &provider_public_key,
    )
    .expect("construct rolled-back-head trust anchor");
    assert!(matches!(
        WalletAuthorityProviderClient::connect_unix(provider.endpoint(), rolled_back_head_trust,)
            .await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));
    let provider_trust = WalletAuthorityProviderTrust::new(
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        &provider_public_key,
    )
    .expect("construct provider trust anchor");
    let provider_client =
        WalletAuthorityProviderClient::connect_unix(provider.endpoint(), provider_trust)
            .await
            .expect("authenticate separate wallet authority provider");
    let qualified_catalog = provider_client
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify routing catalog");
    assert_eq!(qualified_catalog.descriptor(), &fixture.routing_catalog);
    assert_eq!(qualified_catalog.descriptor().generations().len(), 2);
    assert!(qualified_catalog
        .descriptor()
        .generations()
        .iter()
        .all(|generation| generation.chain_instance()
            == &fixture.chain_attestation.binding().expect("chain binding")));
    assert_unissued_chain_catalogs_fail_closed(
        &provider_client,
        &fixture,
        &fixture.routing_catalog,
    )
    .await;
    let routing_qualification_calls_after_deployment = provider
        .routing_qualification_calls()
        .expect("read deployment routing qualification count");
    assert_eq!(
        qualified_catalog
            .routing_policy_object()
            .expect("routing policy")
            .content_ref,
        fixture
            .routing_catalog
            .content_ref()
            .expect("catalog reference")
    );
    let provider_debug = format!("{provider_client:?}");
    assert!(!provider_debug.contains(&provider_public_key));
    assert!(!provider_debug.contains(&provider.endpoint().to_string_lossy().into_owned()));
    let commit_proxy = PostgresCommitFaultProxy::start(&database_url)
        .await
        .expect("start deterministic PostgreSQL commit-fault proxy");
    let proxy_role_urls = role_urls.through_proxy(commit_proxy.database_url());
    let registry = open_activation_registry_admin(
        &role_urls.activation_admin,
        &schema,
        provider_client.clone(),
    )
    .await
    .expect("open activation admin");
    let registry_b = open_activation_registry_admin(
        &role_urls.activation_admin,
        &schema,
        provider_client.clone(),
    )
    .await
    .expect("open independently pooled activation admin");
    let ambiguity_registry = open_activation_registry_admin(
        &proxy_role_urls.activation_admin,
        &schema,
        provider_client.clone(),
    )
    .await
    .expect("open activation admin through commit-fault proxy");
    for fault in [
        ActivationIssuanceFaultPoint::AfterIncarnationInsert,
        ActivationIssuanceFaultPoint::AfterLineageHeadInsert,
        ActivationIssuanceFaultPoint::AfterDomainActivationInsert,
    ] {
        assert!(matches!(
            registry
                .issue_domain_activation_with_fault(&fault_record, &fault_incarnation, fault,)
                .await,
            Err(PostgresEvmWalletError::Unavailable)
        ));
        assert_activation_creation_absent(
            &mut admin_connection,
            &schema,
            &fault_incarnation.wallet_nonce_store_lineage_id,
            fault_record.wallet_nonce_domain.as_str(),
        )
        .await;
    }
    let fault_recovery = registry
        .issue_domain_activation(&fault_record, &fault_incarnation)
        .await
        .expect("retry activation after every internal issuance interruption");
    assert_eq!(fault_recovery.current_schema_record, fault_record);

    let competing_results = tokio::join!(
        registry.issue_domain_activation(&competing_record_a, &competing_incarnation_a),
        registry_b.issue_domain_activation(&competing_record_b, &competing_incarnation_b),
    );
    assert_one_activation_creation_winner(competing_results);
    assert_single_activation_creation(
        &mut admin_connection,
        &schema,
        &[competing_record_a.wallet_nonce_domain.as_str()],
        &[
            competing_incarnation_a
                .wallet_nonce_store_lineage_id
                .as_str(),
            competing_incarnation_b
                .wallet_nonce_store_lineage_id
                .as_str(),
        ],
    )
    .await;

    let sibling_results = tokio::join!(
        registry.issue_domain_activation(&sibling_record_a, &sibling_incarnation_a),
        registry_b.issue_domain_activation(&sibling_record_b, &sibling_incarnation_b),
    );
    assert_one_activation_creation_winner(sibling_results);
    assert_single_activation_creation(
        &mut admin_connection,
        &schema,
        &[
            sibling_record_a.wallet_nonce_domain.as_str(),
            sibling_record_b.wallet_nonce_domain.as_str(),
        ],
        &[sibling_incarnation_a.wallet_nonce_store_lineage_id.as_str()],
    )
    .await;

    let activation_ack_target = commit_proxy
        .arm(CommitFault::CommitAndLoseAcknowledgement, 1)
        .expect("arm activation-registry acknowledgement fault");
    let qualified_activation = ambiguity_registry
        .issue_domain_activation(&fixture.activation_record, &fixture.incarnation)
        .await
        .expect("resolve activation after lost acknowledgement");
    commit_proxy
        .wait_for_intercepts(activation_ack_target)
        .await;
    let (activation_replay_a, activation_replay_b) = tokio::join!(
        registry.issue_domain_activation(&fixture.activation_record, &fixture.incarnation),
        registry_b.issue_domain_activation(&fixture.activation_record, &fixture.incarnation),
    );
    assert_eq!(
        activation_replay_a.expect("resolve concurrent activation replay"),
        qualified_activation
    );
    let secondary_activation = registry_b
        .issue_domain_activation(&fixture.secondary_activation_record, &fixture.incarnation)
        .await
        .expect("issue a second domain on the retained incarnation");
    assert_eq!(
        secondary_activation.current_schema_record,
        fixture.secondary_activation_record
    );
    assert!(matches!(
        registry
            .promote_store_incarnation(&promotion_successor)
            .await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));

    let mut sibling_incarnation = fixture.incarnation.clone();
    sibling_incarnation.physical_target_instance_id =
        "mfm.evm.test/wallet-target-one-sibling".to_owned();
    let mut sibling_target_record = fixture.activation_record.clone();
    sibling_target_record.initial_store_incarnation_ref =
        canonical_wallet_reference(&sibling_incarnation).expect("sibling incarnation reference");
    assert!(matches!(
        registry
            .issue_domain_activation(&sibling_target_record, &sibling_incarnation)
            .await,
        Err(PostgresEvmWalletError::PermanentConflict)
    ));

    let mut competing_lineage_incarnation = sibling_incarnation;
    competing_lineage_incarnation.wallet_nonce_store_lineage_id =
        "mfm.evm.test/competing-wallet-lineage".to_owned();
    competing_lineage_incarnation.physical_target_instance_id =
        "mfm.evm.test/competing-wallet-target".to_owned();
    let mut competing_lineage_record = fixture.activation_record.clone();
    competing_lineage_record.wallet_nonce_store_lineage_id = competing_lineage_incarnation
        .wallet_nonce_store_lineage_id
        .clone();
    competing_lineage_record.initial_store_incarnation_ref =
        canonical_wallet_reference(&competing_lineage_incarnation)
            .expect("competing lineage incarnation reference");
    assert!(matches!(
        registry
            .issue_domain_activation(&competing_lineage_record, &competing_lineage_incarnation)
            .await,
        Err(PostgresEvmWalletError::PermanentConflict)
    ));
    assert_eq!(
        activation_replay_b.expect("resolve concurrent activation replay"),
        qualified_activation
    );
    let mut fabricated_activation = qualified_activation.clone();
    fabricated_activation.registry_issuance_ref = fixture.next_provider_fence_head_ref.clone();
    fabricated_activation
        .validate()
        .expect("fabricated public attestation is structurally well formed");
    assert!(matches!(
        open_wallet_nonce_authority(
            &role_urls.nonce_application,
            &schema,
            fabricated_activation,
            fixture.incarnation.clone(),
            fixture.current_public_head.clone(),
            provider_client.clone(),
        )
        .await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));

    let public = open_activation_registry_public(&role_urls.activation_public, &schema)
        .await
        .expect("open public registry");
    assert_eq!(
        public
            .domain_activation(&fixture.nonce_domain)
            .await
            .expect("read public activation"),
        Some(qualified_activation.clone())
    );
    assert_eq!(
        public
            .current_store_incarnation(&fixture.incarnation.wallet_nonce_store_lineage_id)
            .await
            .expect("read current incarnation"),
        Some(fixture.incarnation.clone())
    );
    assert_eq!(
        public
            .domain_activation(&fixture.secondary_nonce_domain)
            .await
            .expect("read concurrent secondary activation"),
        Some(secondary_activation)
    );
    assert_eq!(
        public
            .current_store_incarnation(
                &competing_lineage_incarnation.wallet_nonce_store_lineage_id,
            )
            .await
            .expect("read absent competing lineage"),
        None
    );

    prove_role_separation(&base_url, &role_urls, &schema, &public_sentinel).await;
    assert!(matches!(
        open_activation_registry_public(&role_urls.activation_public, &schema).await,
        Err(PostgresEvmWalletError::InvalidAuthority)
    ));
    sqlx::query(AssertSqlSafe(format!(
        "DROP TABLE {schema}.wallet_unlisted_future"
    )))
    .execute(&mut admin_connection)
    .await
    .expect("remove hostile unlisted wallet table after constructor rejection");

    let authority_a = Arc::new(
        open_wallet_nonce_authority(
            &role_urls.nonce_application,
            &schema,
            qualified_activation.clone(),
            fixture.incarnation.clone(),
            fixture.current_public_head.clone(),
            provider_client.clone(),
        )
        .await
        .expect("open first sealed nonce authority"),
    );
    let authority_b = Arc::new(
        open_wallet_nonce_authority(
            &role_urls.nonce_application,
            &schema,
            qualified_activation.clone(),
            fixture.incarnation.clone(),
            fixture.current_public_head.clone(),
            provider_client.clone(),
        )
        .await
        .expect("open independently pooled sealed nonce authority"),
    );
    let ambiguity_authority = open_wallet_nonce_authority(
        &proxy_role_urls.nonce_application,
        &schema,
        qualified_activation.clone(),
        fixture.incarnation.clone(),
        fixture.current_public_head.clone(),
        provider_client.clone(),
    )
    .await
    .expect("open authority through commit-fault proxy");
    let sibling_schema_authority = open_wallet_nonce_authority(
        &sibling_schema_role_urls.nonce_application,
        &sibling_schema,
        qualified_activation.clone(),
        fixture.incarnation.clone(),
        fixture.current_public_head.clone(),
        provider_client.clone(),
    )
    .await
    .expect("open copied authority in sibling schema");
    let sibling_database_authority = open_wallet_nonce_authority(
        &sibling_database_role_urls.nonce_application,
        &schema,
        qualified_activation.clone(),
        fixture.incarnation.clone(),
        fixture.current_public_head.clone(),
        provider_client.clone(),
    )
    .await
    .expect("open copied authority in sibling database");
    let probe_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("open wallet persistence probe");

    let state_input = fixture.state_input();
    let request = fixture.reserve_request(qualified_activation.clone(), 7, "intent-one");
    assert_eq!(
        request,
        fixture.reserve_request(qualified_activation.clone(), 7, "intent-one"),
        "stable issuer and token must reproduce the exact permanent request"
    );
    let sibling_issuer_request = fixture.reserve_request_for_principal(
        qualified_activation.clone(),
        7,
        "intent-one",
        stable("mfm.evm.test/sibling-principal"),
    );
    assert_ne!(
        sibling_issuer_request.submission_intent_id,
        request.submission_intent_id
    );
    assert_ne!(
        sibling_issuer_request.reservation_key,
        request.reservation_key
    );

    let first_use_lag =
        fixture.reserve_request(qualified_activation.clone(), 6, "first-use-provider-lag");
    assert!(matches!(
        authority_a.reserve(&state_input, &first_use_lag).await,
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::NonceLineageDiverged)
    ));
    assert_reservation_absent(&probe_pool, &first_use_lag).await;
    assert_domain_absent(&probe_pool, fixture.nonce_domain.as_str()).await;

    let first_use_ahead =
        fixture.reserve_request(qualified_activation.clone(), 8, "first-use-provider-ahead");
    let divergent_ack_target = commit_proxy
        .arm(CommitFault::CommitAndLoseAcknowledgement, 1)
        .expect("arm divergent first-use acknowledgement fault");
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(10),
            ambiguity_authority.reserve(&state_input, &first_use_ahead),
        )
        .await
        .expect("bounded divergent first-use ambiguity resolution"),
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::NonceLineageDiverged)
    ));
    commit_proxy.wait_for_intercepts(divergent_ack_target).await;
    assert_reservation_absent(&probe_pool, &first_use_ahead).await;
    assert_domain_absent(&probe_pool, fixture.nonce_domain.as_str()).await;

    let absent_reservation_target = commit_proxy
        .arm(CommitFault::RollBackBeforeCommit, 1)
        .expect("arm first-use absent reservation acknowledgement fault");
    let ambiguity_reservation = match tokio::time::timeout(
        Duration::from_secs(10),
        ambiguity_authority.reserve(&state_input, &request),
    )
    .await
    .expect("bounded first-use reservation ambiguity resolution")
    {
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved { reservation }) => {
            reservation
        }
        other => panic!("unexpected first-use absent reservation result: {other:?}"),
    };
    commit_proxy
        .wait_for_intercepts(absent_reservation_target)
        .await;
    let (left_reservation, right_reservation) = tokio::join!(
        authority_a.reserve(&state_input, &request),
        authority_b.reserve(&state_input, &request),
    );
    let reservation = match (left_reservation, right_reservation) {
        (
            EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved {
                reservation: left,
            }),
            EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved {
                reservation: right,
            }),
        ) => {
            assert_eq!(left, right);
            left
        }
        other => panic!("unexpected concurrent identical reservation results: {other:?}"),
    };
    assert_eq!(reservation.nonce, 7);
    assert_eq!(reservation, ambiguity_reservation);
    let present_reservation_target = commit_proxy
        .arm(CommitFault::CommitAndLoseAcknowledgement, 1)
        .expect("arm present reservation acknowledgement fault");
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(10),
            ambiguity_authority.reserve(&state_input, &request),
        )
        .await
        .expect("bounded present reservation ambiguity resolution"),
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved {
            reservation: ref retained,
        }) if retained == &reservation
    ));
    commit_proxy
        .wait_for_intercepts(present_reservation_target)
        .await;

    let refreshed_route_ref = EvmWalletReference::from_content_ref(
        fixture
            .redundant_route_generation_ref
            .to_content_ref()
            .expect("redundant route generation reference"),
    );
    let retry = ReserveEvmNonceRequest {
        qualified_floor: QualifiedPendingNonceFloor {
            observed: ObservedPendingNonceFloor {
                nonce_domain: request.nonce_domain.clone(),
                route_generation_ref: refreshed_route_ref,
                pending_nonce: 6,
            },
            pending_floor_policy_ref: fixture.common_ref.clone(),
        },
        ..request.clone()
    };
    assert!(matches!(
        authority_a
            .reserve(&fixture.refreshed_state_input(), &retry)
            .await,
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved {
            reservation: ref retained
        }) if retained == &reservation
    ));
    assert_eq!(retry.submission_intent_id, request.submission_intent_id);
    assert_eq!(retry.reservation_key, request.reservation_key);

    let (activation_request, expected_unsigned_digest) =
        fixture.activation_request(&request, &reservation);
    let busy_ack_target = commit_proxy
        .arm_held_lost_acknowledgement()
        .expect("arm held busy disposition acknowledgement fault");
    let (busy_result, ambiguity_candidate) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(30),
            ambiguity_authority.reserve(&state_input, &sibling_issuer_request),
        ),
        async {
            commit_proxy
                .wait_for_held_lost_acknowledgements(busy_ack_target)
                .await;
            assert_reservation_absent(&probe_pool, &sibling_issuer_request).await;
            assert_candidate_absent(&probe_pool, &activation_request).await;
            assert_domain_high_water(&probe_pool, fixture.nonce_domain.as_str(), 7).await;

            let absent_activation_target = commit_proxy
                .arm(CommitFault::RollBackBeforeCommit, 1)
                .expect("arm absent candidate acknowledgement fault");
            let candidate = match tokio::time::timeout(
                Duration::from_secs(10),
                ambiguity_authority.activate_candidate(&state_input, &activation_request),
            )
            .await
            .expect("bounded absent candidate ambiguity resolution")
            {
                EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated {
                    candidate,
                }) => candidate,
                other => panic!("unexpected absent candidate result: {other:?}"),
            };
            commit_proxy
                .wait_for_intercepts(absent_activation_target)
                .await;
            commit_proxy.release_held_transactions();
            candidate
        },
    );
    assert!(matches!(
        busy_result.expect("bounded busy disposition ambiguity resolution"),
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::NonceDomainBusy)
    ));
    assert_reservation_absent(&probe_pool, &sibling_issuer_request).await;
    assert_domain_high_water(&probe_pool, fixture.nonce_domain.as_str(), 7).await;

    let (left_activation, right_activation) = tokio::join!(
        authority_a.activate_candidate(&state_input, &activation_request),
        authority_b.activate_candidate(&state_input, &activation_request),
    );
    let active_candidate = match (left_activation, right_activation) {
        (
            EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated {
                candidate: left,
            }),
            EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated {
                candidate: right,
            }),
        ) => {
            assert_eq!(left, right);
            left
        }
        other => panic!("unexpected concurrent activation results: {other:?}"),
    };
    assert_eq!(
        active_candidate
            .attested_candidate
            .unsigned_candidate_digest,
        expected_unsigned_digest
    );
    assert_eq!(active_candidate, ambiguity_candidate);
    let retained_candidate: (String, String) = sqlx::query_as(
        "SELECT request_json, active_candidate_json \
         FROM wallet_nonce_candidates \
         WHERE semantic_candidate_operation_key = $1",
    )
    .bind(activation_request.candidate_operation_key.as_str())
    .fetch_one(&probe_pool)
    .await
    .expect("read retained candidate operation");
    let mut refreshed_evidence_activation = activation_request.clone();
    refreshed_evidence_activation
        .next_candidate
        .signer_attestation_ref = fixture.next_lineage_head.public_lineage_head_ref.clone();
    assert_ne!(refreshed_evidence_activation, activation_request);
    let present_activation_target = commit_proxy
        .arm(CommitFault::CommitAndLoseAcknowledgement, 1)
        .expect("arm present candidate acknowledgement fault");
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(10),
            ambiguity_authority
                .activate_candidate(&state_input, &refreshed_evidence_activation),
        )
        .await
        .expect("bounded present candidate ambiguity resolution"),
        EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated {
            candidate: ref retained,
        }) if retained == &active_candidate
    ));
    commit_proxy
        .wait_for_intercepts(present_activation_target)
        .await;
    let mut conflicting_descriptor = activation_request.clone();
    conflicting_descriptor
        .next_candidate
        .candidate_descriptor_ref = fixture.next_lineage_head.public_lineage_head_ref.clone();
    assert!(matches!(
        authority_a
            .activate_candidate(&state_input, &conflicting_descriptor)
            .await,
        EffectAdapterCompletion::IntegrityFault(_)
    ));
    let mut conflicting_signer = activation_request.clone();
    conflicting_signer.next_candidate.semantic_signer_id =
        "mfm.evm.test/conflicting-semantic-signer".to_owned();
    assert!(matches!(
        authority_a
            .activate_candidate(&state_input, &conflicting_signer)
            .await,
        EffectAdapterCompletion::IntegrityFault(_)
    ));
    let mut conflicting_hash = activation_request.clone();
    conflicting_hash.next_candidate.transaction_hash = format!("{:#x}", B256::repeat_byte(0x73));
    assert!(matches!(
        authority_a
            .activate_candidate(&state_input, &conflicting_hash)
            .await,
        EffectAdapterCompletion::IntegrityFault(_)
    ));
    assert_eq!(
        sqlx::query_as::<_, (String, String)>(
            "SELECT request_json, active_candidate_json \
             FROM wallet_nonce_candidates \
             WHERE semantic_candidate_operation_key = $1",
        )
        .bind(activation_request.candidate_operation_key.as_str())
        .fetch_one(&probe_pool)
        .await
        .expect("re-read retained candidate operation"),
        retained_candidate
    );

    let (replacement_request, replacement_unsigned_digest) = fixture.replacement_request(
        &request,
        &reservation,
        std::slice::from_ref(&active_candidate),
    );
    let mut skipped_replacement = replacement_request.clone();
    skipped_replacement.next_candidate.candidate_ordinal = 2;
    skipped_replacement.candidate_operation_key = derive_evm_candidate_operation_key(
        &reservation.semantic_reservation_key,
        skipped_replacement.next_candidate.candidate_ordinal,
    )
    .expect("skipped candidate key");
    skipped_replacement.activation_permit = CandidateActivationPermit::Replacement {
        predecessor_activation_ref: fixture.common_ref.clone(),
        predecessor_ordinal: 1,
        exact_next_ordinal: 2,
        replacement_policy_ref: evm_wallet_nonce_policy_ref().expect("replacement policy"),
        eligibility_ref: mfm_journal::structured::domain_content_digest(
            "mfm.evm.test/skipped-candidate-eligibility.v1",
            &replacement_request,
        )
        .expect("skipped candidate eligibility")
        .as_str()
        .to_owned(),
    };
    let mut wrong_predecessor = replacement_request.clone();
    if let CandidateActivationPermit::Replacement {
        predecessor_activation_ref,
        ..
    } = &mut wrong_predecessor.activation_permit
    {
        *predecessor_activation_ref = fixture.common_ref.clone();
    }
    assert!(matches!(
        authority_a
            .activate_candidate(&state_input, &wrong_predecessor)
            .await,
        EffectAdapterCompletion::Returned(ActivateCandidateResponse::CandidateProgressionConflict)
    ));
    assert_candidate_absent(&probe_pool, &wrong_predecessor).await;

    let mut wrong_policy = replacement_request.clone();
    if let CandidateActivationPermit::Replacement {
        replacement_policy_ref,
        ..
    } = &mut wrong_policy.activation_permit
    {
        *replacement_policy_ref = fixture.next_provider_fence_head_ref.clone();
    }
    assert!(matches!(
        authority_a
            .activate_candidate(&state_input, &wrong_policy)
            .await,
        EffectAdapterCompletion::Returned(ActivateCandidateResponse::CandidateProgressionConflict)
    ));
    assert_candidate_absent(&probe_pool, &wrong_policy).await;

    let mut wrong_eligibility = replacement_request.clone();
    if let CandidateActivationPermit::Replacement {
        eligibility_ref, ..
    } = &mut wrong_eligibility.activation_permit
    {
        *eligibility_ref = mfm_journal::structured::domain_content_digest(
            "mfm.evm.test/forged-candidate-eligibility.v1",
            &replacement_request,
        )
        .expect("forged candidate eligibility")
        .as_str()
        .to_owned();
    }
    assert!(matches!(
        authority_a
            .activate_candidate(&state_input, &wrong_eligibility)
            .await,
        EffectAdapterCompletion::Returned(ActivateCandidateResponse::CandidateProgressionConflict)
    ));
    assert_candidate_absent(&probe_pool, &wrong_eligibility).await;

    let progression_ack_target = commit_proxy
        .arm_held_lost_acknowledgement()
        .expect("arm candidate-progression acknowledgement fault");
    let (progression_result, replacement_candidate) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(30),
            ambiguity_authority.activate_candidate(&state_input, &skipped_replacement),
        ),
        async {
            commit_proxy
                .wait_for_held_lost_acknowledgements(progression_ack_target)
                .await;
            assert_candidate_absent(&probe_pool, &skipped_replacement).await;
            assert_candidate_absent(&probe_pool, &replacement_request).await;
            let replacement_state_input = fixture.refreshed_state_input();
            let (replacement_a, replacement_b) = tokio::join!(
                authority_a.activate_candidate(&state_input, &replacement_request),
                authority_b.activate_candidate(&replacement_state_input, &replacement_request),
            );
            let candidate = match (replacement_a, replacement_b) {
                (
                    EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated {
                        candidate: left,
                    }),
                    EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated {
                        candidate: right,
                    }),
                ) => {
                    assert_eq!(left, right);
                    left
                }
                other => panic!("unexpected concurrent replacement results: {other:?}"),
            };
            commit_proxy.release_held_transactions();
            candidate
        },
    );
    assert!(matches!(
        progression_result.expect("bounded candidate-progression ambiguity resolution"),
        EffectAdapterCompletion::Returned(ActivateCandidateResponse::CandidateProgressionConflict)
    ));
    assert_candidate_absent(&probe_pool, &skipped_replacement).await;
    assert_eq!(
        replacement_candidate.attested_candidate.candidate_ordinal,
        1
    );
    assert_eq!(
        replacement_candidate
            .attested_candidate
            .unsigned_candidate_digest,
        replacement_unsigned_digest
    );

    let status_request = ReadEvmWalletNonceStatusRequest {
        nonce_domain: request.nonce_domain.clone(),
        domain_activation_attestation: qualified_activation.clone(),
        semantic_reservation_key: request.reservation_key.clone(),
        submission_intent_id: request.submission_intent_id.clone(),
        transaction_intent_digest: request.transaction_intent.digest().to_owned(),
        candidate_family_ref: request.candidate_family.digest().to_owned(),
    };
    assert!(matches!(
        authority_a.read_status(&state_input, &status_request).await,
        ReadAdapterCompletion::Returned(WalletNonceStatus::Reserved {
            ref activated_candidates,
            current_candidate: Some(ref current),
            ..
        }) if activated_candidates == &vec![active_candidate.clone(), replacement_candidate.clone()]
            && current == &replacement_candidate
    ));

    let completion_request = fixture.completion_request(&reservation, &active_candidate);
    let forged_hash = format!("{:#x}", B256::repeat_byte(0xa1));
    let mut forged_transaction = completion_request.clone();
    forged_transaction
        .canonical_terminal_outcome
        .transaction_hash = forged_hash.clone();
    if let EvmTransactionLookupObservation::Found {
        transaction_hash, ..
    } = &mut forged_transaction.terminal_witnesses.transaction
    {
        *transaction_hash = forged_hash.clone();
    }
    if let EvmReceiptLookupObservation::Found {
        transaction_hash, ..
    } = &mut forged_transaction.terminal_witnesses.receipt
    {
        *transaction_hash = forged_hash;
    }
    assert!(
        forged_transaction.validate().is_ok(),
        "the domain-only closure cannot resolve the retained candidate prefix"
    );

    let mut forged_receipt_hash = completion_request.clone();
    if let EvmReceiptLookupObservation::Found { block_hash, .. } =
        &mut forged_receipt_hash.terminal_witnesses.receipt
    {
        *block_hash = format!("{:#x}", B256::repeat_byte(0xa2));
    }
    let mut forged_block = completion_request.clone();
    forged_block.terminal_witnesses.inclusion_block.block_number = "100".to_owned();
    let mut forged_head = completion_request.clone();
    forged_head.terminal_witnesses.finalized_head.block_number = "100".to_owned();
    let mut forged_status = completion_request.clone();
    if let EvmReceiptLookupObservation::Found { status, .. } =
        &mut forged_status.terminal_witnesses.receipt
    {
        *status = 0;
    }
    let mut forged_assurance = completion_request.clone();
    forged_assurance
        .canonical_terminal_outcome
        .terminal_assurance_contract_ref = fixture.common_ref.clone();
    forged_assurance
        .terminal_witnesses
        .terminal_assurance_contract_ref = fixture.common_ref.clone();
    assert!(
        forged_assurance.validate().is_ok(),
        "the domain-only closure cannot resolve the retained intent"
    );
    let mut forged_projection = completion_request.clone();
    forged_projection
        .canonical_terminal_outcome
        .canonical_public_result = "{\"conflict\":true}".to_owned();
    forged_projection.terminal_witnesses.canonical_public_result = "{\"conflict\":true}".to_owned();

    for (label, forged) in [
        ("transaction", forged_transaction),
        ("receipt hash", forged_receipt_hash),
        ("inclusion block", forged_block),
        ("finalized head", forged_head),
        ("execution status", forged_status),
        ("terminal assurance", forged_assurance),
        ("public projection", forged_projection),
    ] {
        assert!(
            matches!(
                authority_a.complete(&state_input, &forged).await,
                EffectAdapterCompletion::IntegrityFault(_)
            ),
            "accepted forged completion {label}"
        );
        let retained = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM wallet_nonce_completions WHERE semantic_completion_key = $1",
        )
        .bind(completion_request.completion_key.as_str())
        .fetch_one(&probe_pool)
        .await
        .expect("count absent completion after hostile request");
        assert_eq!(retained, 0, "forged {label} committed completion state");
    }

    let absent_completion_target = commit_proxy
        .arm(CommitFault::RollBackBeforeCommit, 1)
        .expect("arm absent completion acknowledgement fault");
    let ambiguity_completion = match tokio::time::timeout(
        Duration::from_secs(10),
        ambiguity_authority.complete(&state_input, &completion_request),
    )
    .await
    .expect("bounded absent completion ambiguity resolution")
    {
        EffectAdapterCompletion::Returned(CompleteWalletNonceResponse::Completed {
            completion,
        }) => completion,
        other => panic!("unexpected absent completion result: {other:?}"),
    };
    commit_proxy
        .wait_for_intercepts(absent_completion_target)
        .await;
    let (left_completion, right_completion) = tokio::join!(
        authority_a.complete(&state_input, &completion_request),
        authority_b.complete(&state_input, &completion_request),
    );
    let completion = match (left_completion, right_completion) {
        (
            EffectAdapterCompletion::Returned(CompleteWalletNonceResponse::Completed {
                completion: left,
            }),
            EffectAdapterCompletion::Returned(CompleteWalletNonceResponse::Completed {
                completion: right,
            }),
        ) => {
            assert_eq!(left, right);
            left
        }
        other => panic!("unexpected concurrent completion results: {other:?}"),
    };
    assert_eq!(completion, ambiguity_completion);
    assert!(matches!(
        authority_a.complete(&state_input, &completion_request).await,
        EffectAdapterCompletion::Returned(CompleteWalletNonceResponse::Completed {
            completion: ref replayed,
        }) if replayed == &completion
    ));
    let retained_completion: (String, String) = sqlx::query_as(
        "SELECT request_json, completion_json \
         FROM wallet_nonce_completions \
         WHERE semantic_completion_key = $1",
    )
    .bind(completion_request.completion_key.as_str())
    .fetch_one(&probe_pool)
    .await
    .expect("read retained completion operation");
    let mut refreshed_witness_completion = completion_request.clone();
    refreshed_witness_completion
        .terminal_witnesses
        .finalized_head = EvmFinalizedHeadObservation {
        block_number: "103".to_owned(),
        block_hash: format!("{:#x}", B256::repeat_byte(0x74)),
    };
    let present_completion_target = commit_proxy
        .arm(CommitFault::CommitAndLoseAcknowledgement, 1)
        .expect("arm present completion acknowledgement fault");
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(10),
            ambiguity_authority.complete(&state_input, &refreshed_witness_completion),
        )
        .await
        .expect("bounded present completion ambiguity resolution"),
        EffectAdapterCompletion::Returned(CompleteWalletNonceResponse::Completed {
            completion: ref retained,
        }) if retained == &completion
    ));
    commit_proxy
        .wait_for_intercepts(present_completion_target)
        .await;
    let mut conflicting_completion = completion_request.clone();
    conflicting_completion
        .canonical_terminal_outcome
        .execution_disposition = ExecutionDisposition::Reverted;
    conflicting_completion
        .canonical_terminal_outcome
        .canonical_public_result = "{\"execution_disposition\":\"reverted\"}".to_owned();
    if let EvmReceiptLookupObservation::Found { status, .. } =
        &mut conflicting_completion.terminal_witnesses.receipt
    {
        *status = 0;
    }
    conflicting_completion
        .terminal_witnesses
        .canonical_public_result = "{\"execution_disposition\":\"reverted\"}".to_owned();
    assert_ne!(conflicting_completion, completion_request);
    assert!(matches!(
        authority_a
            .complete(&state_input, &conflicting_completion)
            .await,
        EffectAdapterCompletion::IntegrityFault(_)
    ));
    assert_eq!(
        sqlx::query_as::<_, (String, String)>(
            "SELECT request_json, completion_json \
             FROM wallet_nonce_completions \
             WHERE semantic_completion_key = $1",
        )
        .bind(completion_request.completion_key.as_str())
        .fetch_one(&probe_pool)
        .await
        .expect("re-read retained completion operation"),
        retained_completion
    );

    assert!(matches!(
        authority_a.read_status(&state_input, &status_request).await,
        ReadAdapterCompletion::Returned(WalletNonceStatus::Completed {
            completion: ref retained,
            ref activated_candidates,
            ..
        }) if retained == &completion
            && activated_candidates == &vec![active_candidate.clone(), replacement_candidate.clone()]
            && retained.canonical_terminal_outcome.winning_candidate_ordinal == 0
    ));

    let provider_ahead_probe =
        fixture.reserve_request(qualified_activation.clone(), 9, "provider-ahead-probe");
    assert!(matches!(
        authority_a
            .reserve(&state_input, &provider_ahead_probe)
            .await,
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::NonceLineageDiverged)
    ));
    assert_reservation_absent(&probe_pool, &provider_ahead_probe).await;
    assert_domain_high_water(&probe_pool, fixture.nonce_domain.as_str(), 7).await;

    let later_provider_ahead =
        fixture.reserve_request(qualified_activation.clone(), 9, "later-provider-ahead");
    let divergence_ack_target = commit_proxy
        .arm_held_lost_acknowledgement()
        .expect("arm held provider-ahead acknowledgement fault");
    let (later_provider_ahead_result, sibling_issuer_reservation) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(30),
            ambiguity_authority.reserve(&state_input, &later_provider_ahead),
        ),
        async {
            commit_proxy
                .wait_for_held_lost_acknowledgements(divergence_ack_target)
                .await;
            assert_reservation_absent(&probe_pool, &later_provider_ahead).await;
            assert_domain_high_water(&probe_pool, fixture.nonce_domain.as_str(), 7).await;
            let reservation = match authority_a
                .reserve(&fixture.refreshed_state_input(), &sibling_issuer_request)
                .await
            {
                EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved {
                    reservation,
                }) => reservation,
                other => panic!("unexpected sibling-issuer reservation result: {other:?}"),
            };
            commit_proxy.release_held_transactions();
            reservation
        },
    );
    assert!(matches!(
        later_provider_ahead_result.expect("bounded provider-ahead ambiguity resolution"),
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::NonceDomainBusy)
    ));
    assert_reservation_absent(&probe_pool, &later_provider_ahead).await;
    assert_domain_high_water(&probe_pool, fixture.nonce_domain.as_str(), 8).await;
    assert_eq!(sibling_issuer_reservation.nonce, 8);
    let (sibling_issuer_activation, _) =
        fixture.activation_request(&sibling_issuer_request, &sibling_issuer_reservation);
    let sibling_issuer_candidate = match authority_b
        .activate_candidate(&fixture.refreshed_state_input(), &sibling_issuer_activation)
        .await
    {
        EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated { candidate }) => {
            candidate
        }
        other => panic!("unexpected sibling-issuer activation result: {other:?}"),
    };
    let sibling_issuer_completion =
        fixture.completion_request(&sibling_issuer_reservation, &sibling_issuer_candidate);
    let (sibling_replacement_request, _) = fixture.replacement_request(
        &sibling_issuer_request,
        &sibling_issuer_reservation,
        std::slice::from_ref(&sibling_issuer_candidate),
    );
    let sibling_race_state_input = fixture.refreshed_state_input();
    let (sibling_replacement_result, sibling_completion_result) = tokio::join!(
        authority_a.activate_candidate(&sibling_race_state_input, &sibling_replacement_request),
        authority_b.complete(&state_input, &sibling_issuer_completion),
    );
    let sibling_replacement_won = match sibling_replacement_result {
        EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated { .. }) => true,
        EffectAdapterCompletion::Returned(
            ActivateCandidateResponse::CandidateProgressionConflict,
        ) => false,
        other => panic!("unexpected activation/completion race activation result: {other:?}"),
    };
    assert!(matches!(
        sibling_completion_result,
        EffectAdapterCompletion::Returned(CompleteWalletNonceResponse::Completed { .. })
    ));
    let sibling_status_request = ReadEvmWalletNonceStatusRequest {
        nonce_domain: sibling_issuer_request.nonce_domain.clone(),
        domain_activation_attestation: qualified_activation.clone(),
        semantic_reservation_key: sibling_issuer_request.reservation_key.clone(),
        submission_intent_id: sibling_issuer_request.submission_intent_id.clone(),
        transaction_intent_digest: sibling_issuer_request
            .transaction_intent
            .digest()
            .to_owned(),
        candidate_family_ref: sibling_issuer_request.candidate_family.digest().to_owned(),
    };
    assert!(matches!(
        authority_a
            .read_status(&state_input, &sibling_status_request)
            .await,
        ReadAdapterCompletion::Returned(WalletNonceStatus::Completed {
            ref activated_candidates,
            ref completion,
            ..
        }) if activated_candidates.len() == if sibling_replacement_won { 2 } else { 1 }
            && completion.canonical_terminal_outcome.winning_candidate_ordinal == 0
    ));
    prove_high_water_cannot_reset(
        &role_urls.nonce_application,
        fixture.nonce_domain.as_str(),
        8,
    )
    .await;
    assert_no_provider_wire_evidence(
        &probe_pool,
        &provider_public_key,
        &provider.endpoint().to_string_lossy(),
    )
    .await;
    prove_copied_target_rejected(
        &sibling_schema_authority,
        &state_input,
        &status_request,
        &request,
        &activation_request,
        &completion_request,
    )
    .await;
    prove_copied_target_rejected(
        &sibling_database_authority,
        &state_input,
        &status_request,
        &request,
        &activation_request,
        &completion_request,
    )
    .await;
    assert_wallet_nonce_tables_empty(&sibling_schema_url).await;
    assert_wallet_nonce_tables_empty(&sibling_database_url).await;

    let competing_left = fixture.reserve_request(qualified_activation.clone(), 8, "intent-two");
    let competing_right = fixture.reserve_request(qualified_activation.clone(), 8, "intent-three");
    let (left_competition, right_competition) = tokio::join!(
        authority_a.reserve(&state_input, &competing_left),
        authority_b.reserve(&state_input, &competing_right),
    );
    let (next_reservation, next_request, losing_request) =
        match (left_competition, right_competition) {
            (
                EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved {
                    reservation,
                }),
                EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::NonceDomainBusy),
            ) => (reservation, competing_left, competing_right),
            (
                EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::NonceDomainBusy),
                EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved {
                    reservation,
                }),
            ) => (reservation, competing_right, competing_left),
            other => panic!("unexpected concurrent competing reservation results: {other:?}"),
        };
    assert_eq!(next_reservation.nonce, 9);
    assert_reservation_absent(&probe_pool, &losing_request).await;
    let (next_activation_request, _) = fixture.activation_request(&next_request, &next_reservation);
    let next_candidate = match authority_a
        .activate_candidate(&state_input, &next_activation_request)
        .await
    {
        EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated { candidate }) => {
            candidate
        }
        other => panic!("unexpected competing-winner activation result: {other:?}"),
    };
    let next_completion_request = fixture.completion_request(&next_reservation, &next_candidate);
    assert!(matches!(
        authority_a
            .complete(&state_input, &next_completion_request)
            .await,
        EffectAdapterCompletion::Returned(CompleteWalletNonceResponse::Completed { .. })
    ));

    assert!(matches!(
        registry
            .promote_store_incarnation(&promotion_successor)
            .await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));
    assert_eq!(
        provider
            .routing_qualification_calls()
            .expect("read post-operation routing qualification count"),
        routing_qualification_calls_after_deployment,
        "normal status and mutation must perform zero chain-registry qualifications"
    );

    provider
        .crash()
        .expect("crash the provider before old-target fencing");
    drop(provider);
    let mut provider = ProviderProcess::spawn(
        env!("CARGO_BIN_EXE_mfm-storage-wallet-authority-provider"),
        provider_config.clone(),
    )
    .expect("restart the still-open provider before fencing");
    assert_eq!(
        provider.public_key_hex(),
        provider_public_key,
        "provider process restart must retain the qualified external authority key"
    );
    assert_eq!(
        public
            .current_store_incarnation(&fixture.incarnation.wallet_nonce_store_lineage_id)
            .await
            .expect("read lineage after pre-fence crash"),
        Some(fixture.incarnation.clone())
    );
    assert!(matches!(
        authority_a.read_status(&state_input, &status_request).await,
        ReadAdapterCompletion::Returned(WalletNonceStatus::Completed { .. })
    ));

    let held_resolution_request =
        fixture.reserve_request(qualified_activation.clone(), 7, "held-resolution");
    let held_resolution_target = commit_proxy
        .arm(CommitFault::HoldTransactionBeforeCommit, 1)
        .expect("arm live-XID resolution refusal");
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(10),
            ambiguity_authority.reserve(&state_input, &held_resolution_request),
        )
        .await
        .expect("resolution refuses an original live transaction"),
        EffectAdapterCompletion::EntryUnknown(_)
    ));
    commit_proxy
        .wait_for_intercepts(held_resolution_target)
        .await;
    commit_proxy.release_held_transactions();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if provider.active_leases().expect("read provider lease count") == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the terminated original transaction drains its resolution lease");
    let held_resolution_reservation = match ambiguity_authority
        .reserve(&state_input, &held_resolution_request)
        .await
    {
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved { reservation }) => {
            reservation
        }
        other => panic!("unexpected held-resolution replay result: {other:?}"),
    };
    let held_resolution_status_request = ReadEvmWalletNonceStatusRequest {
        nonce_domain: held_resolution_request.nonce_domain.clone(),
        domain_activation_attestation: qualified_activation.clone(),
        semantic_reservation_key: held_resolution_request.reservation_key.clone(),
        submission_intent_id: held_resolution_request.submission_intent_id.clone(),
        transaction_intent_digest: held_resolution_request
            .transaction_intent
            .digest()
            .to_owned(),
        candidate_family_ref: held_resolution_request.candidate_family.digest().to_owned(),
    };
    let pre_hold_status = authority_a
        .read_status(&state_input, &held_resolution_status_request)
        .await;
    assert!(
        matches!(
            &pre_hold_status,
            ReadAdapterCompletion::Returned(WalletNonceStatus::Reserved { reservation, .. })
                if reservation == &held_resolution_reservation
        ),
        "reconciled held-resolution aggregate is invalid before lease holds: {pre_hold_status:?}"
    );

    provider
        .hold_next_affine_leases(2)
        .expect("arm bounded provider lease holds");
    let held_read_authority = Arc::clone(&authority_a);
    let held_read_state = state_input.clone();
    let held_read_request = held_resolution_status_request.clone();
    let held_read = tokio::spawn(async move {
        held_read_authority
            .read_status(&held_read_state, &held_read_request)
            .await
    });
    let held_write_authority = Arc::clone(&authority_b);
    let held_write_state = state_input.clone();
    let held_write_request = fixture.reserve_request(qualified_activation.clone(), 7, "held-write");
    let held_write = tokio::spawn(async move {
        held_write_authority
            .reserve(&held_write_state, &held_write_request)
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if provider
                .entered_affine_lease_holds()
                .expect("read entered provider lease-hold count")
                >= 2
            {
                assert!(
                    provider.active_leases().expect("read provider lease count") >= 2,
                    "every entered provider lease hold remains an active affine lease"
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("read and write leases become live");

    let mut revoke = tokio::task::spawn_blocking(move || {
        let result = provider.revoke();
        (provider, result)
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !revoke.is_finished(),
        "revocation must wait for every live affine lease"
    );

    let held_read_result = tokio::time::timeout(Duration::from_secs(10), held_read)
        .await
        .expect("held read finishes")
        .expect("held read task joins");
    assert!(matches!(
        &held_read_result,
        ReadAdapterCompletion::Returned(WalletNonceStatus::Reserved { reservation, .. })
            if reservation == &held_resolution_reservation
    ));
    let held_write_result = tokio::time::timeout(Duration::from_secs(10), held_write)
        .await
        .expect("held write finishes")
        .expect("held write task joins");
    assert!(matches!(
        held_write_result,
        EffectAdapterCompletion::SupersededBeforeEntry(ref evidence)
            if evidence == &fixture.next_lineage_head
    ));
    let (mut provider, revoke_result) = tokio::time::timeout(Duration::from_secs(10), &mut revoke)
        .await
        .expect("revocation finishes after both bounded lease holds drain")
        .expect("revocation task joins");
    revoke_result.expect("irrevocably revoke and drain the old provider target");
    let stale_request = fixture.reserve_request(qualified_activation.clone(), 7, "intent-three");
    let stale_evidence = match authority_a.reserve(&state_input, &stale_request).await {
        EffectAdapterCompletion::SupersededBeforeEntry(evidence) => evidence,
        other => panic!("unexpected stale authority result: {other:?}"),
    };
    assert_eq!(stale_evidence, fixture.next_lineage_head);
    assert_eq!(
        authority_a.supersession_head(&stale_evidence).await,
        Some(fixture.next_public_head.clone())
    );
    assert!(matches!(
        authority_a
            .activate_candidate(&state_input, &activation_request)
            .await,
        EffectAdapterCompletion::SupersededBeforeEntry(ref evidence)
            if evidence == &fixture.next_lineage_head
    ));
    assert!(matches!(
        authority_a.complete(&state_input, &completion_request).await,
        EffectAdapterCompletion::SupersededBeforeEntry(ref evidence)
            if evidence == &fixture.next_lineage_head
    ));
    assert!(matches!(
        authority_a.read_status(&state_input, &status_request).await,
        ReadAdapterCompletion::SafeFailure(EvmSubmissionFailure::NonceAuthorityUnavailable)
    ));
    assert!(
        open_wallet_nonce_authority(
            &role_urls.nonce_application,
            &schema,
            qualified_activation.clone(),
            fixture.next_incarnation.clone(),
            fixture.next_public_head.clone(),
            provider_client.clone(),
        )
        .await
        .is_err(),
        "the replacement must remain closed after fencing and before registry CAS"
    );

    let _discarded_prefix_digest = provider
        .authorize_promotion()
        .expect("authorize the drained configured successor");
    provider
        .forget_hydrated_prefix()
        .expect("simulate lost closed-prefix hydration proof");
    assert!(matches!(
        registry
            .promote_store_incarnation(&promotion_successor)
            .await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));
    let captured_prefix_digest = provider
        .authorize_promotion()
        .expect("recapture the complete closed prefix after proof loss");
    assert_hostile_restore_prefix_rejected(
        &mut admin_connection,
        &schema,
        &registry,
        &promotion_successor,
        &fixture,
        &qualified_activation,
        &reservation,
        &active_candidate,
        &completion,
    )
    .await;

    provider
        .crash()
        .expect("crash after fencing and complete-prefix capture but before registry CAS");
    drop(provider);
    assert_eq!(
        public
            .current_store_incarnation(&fixture.incarnation.wallet_nonce_store_lineage_id)
            .await
            .expect("read lineage after post-fence pre-CAS crash"),
        Some(fixture.incarnation.clone())
    );

    let crash_restore_completion_key = "mfm.evm.test/crash-restored-completion-key";
    set_replication_role(&mut admin_connection, "replica").await;
    let crash_tamper = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {schema}.wallet_nonce_completions \
         SET semantic_completion_key = $2 WHERE semantic_completion_key = $1"
    )))
    .bind(completion.semantic_completion_key.as_str())
    .bind(crash_restore_completion_key)
    .execute(&mut admin_connection)
    .await
    .expect("tamper the closed prefix after the provider process crashed");
    assert_eq!(crash_tamper.rows_affected(), 1);
    set_replication_role(&mut admin_connection, "origin").await;
    let tampered_restart_config = provider_config
        .clone()
        .with_startup_state(ProviderStartupState::FencedPromotionReady {
            captured_prefix_digest: captured_prefix_digest.clone(),
        })
        .expect("restart from the externally retained fenced prefix");
    assert!(
        ProviderProcess::spawn(
            env!("CARGO_BIN_EXE_mfm-storage-wallet-authority-provider"),
            tampered_restart_config,
        )
        .is_err(),
        "checkpoint divergence must reject the child before readiness"
    );
    set_replication_role(&mut admin_connection, "replica").await;
    let crash_restore = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {schema}.wallet_nonce_completions \
         SET semantic_completion_key = $2 WHERE semantic_completion_key = $1"
    )))
    .bind(crash_restore_completion_key)
    .bind(completion.semantic_completion_key.as_str())
    .execute(&mut admin_connection)
    .await
    .expect("restore the closed prefix after crash-tamper rejection");
    assert_eq!(crash_restore.rows_affected(), 1);
    set_replication_role(&mut admin_connection, "origin").await;

    let post_cas_crash_config = provider_config
        .clone()
        .with_startup_state(ProviderStartupState::FencedPromotionReady {
            captured_prefix_digest,
        })
        .expect("restart from the retained fenced prefix")
        .with_promotion_crash_point(ProviderPromotionCrashPoint::AfterCasBeforeOpen)
        .expect("arm post-CAS pre-open process crash");
    let provider = ProviderProcess::spawn(
        env!("CARGO_BIN_EXE_mfm-storage-wallet-authority-provider"),
        post_cas_crash_config,
    )
    .expect("restart fenced provider for promotion CAS");
    assert_eq!(provider.public_key_hex(), provider_public_key);
    assert!(matches!(
        registry
            .promote_store_incarnation(&promotion_successor)
            .await,
        Err(PostgresEvmWalletError::Unavailable)
    ));
    assert_eq!(
        public
            .current_store_incarnation(&fixture.incarnation.wallet_nonce_store_lineage_id)
            .await
            .expect("read committed promotion after pre-open crash"),
        Some(fixture.next_incarnation.clone())
    );
    assert!(
        open_wallet_nonce_authority(
            &role_urls.nonce_application,
            &schema,
            qualified_activation.clone(),
            fixture.next_incarnation.clone(),
            fixture.next_public_head.clone(),
            provider_client.clone(),
        )
        .await
        .is_err(),
        "a process crash after CAS must leave the replacement unavailable until exact recovery"
    );
    drop(provider);

    let post_open_crash_config = provider_config
        .clone()
        .with_startup_state(ProviderStartupState::PromotedClosed)
        .expect("restart with the committed replacement still closed")
        .with_promotion_crash_point(ProviderPromotionCrashPoint::AfterOpenBeforeConfirmation)
        .expect("arm post-open pre-confirmation process crash");
    let provider = ProviderProcess::spawn(
        env!("CARGO_BIN_EXE_mfm-storage-wallet-authority-provider"),
        post_open_crash_config,
    )
    .expect("restart closed promoted provider");
    assert!(matches!(
        registry
            .promote_store_incarnation(&promotion_successor)
            .await,
        Err(PostgresEvmWalletError::Unavailable)
    ));
    drop(provider);

    let recovered_provider_config = provider_config
        .clone()
        .with_startup_state(ProviderStartupState::PromotedOpen)
        .expect("restart the durably opened promoted provider");
    let provider = ProviderProcess::spawn(
        env!("CARGO_BIN_EXE_mfm-storage-wallet-authority-provider"),
        recovered_provider_config,
    )
    .expect("restart after post-open process crash");
    assert_eq!(provider.public_key_hex(), provider_public_key);
    let promotion = registry
        .promote_store_incarnation(&promotion_successor)
        .await
        .expect("resolve exact promotion after both post-CAS crash points");
    assert_eq!(promotion.writer_epoch, 2);
    assert_eq!(
        registry
            .promote_store_incarnation(&promotion_successor)
            .await
            .expect("resolve exact promotion replay"),
        promotion
    );
    assert_eq!(
        registry
            .issue_domain_activation(&fixture.activation_record, &fixture.incarnation)
            .await
            .expect("resolve the original activation after promotion"),
        qualified_activation
    );

    let successor_authority = open_wallet_nonce_authority(
        &role_urls.nonce_application,
        &schema,
        qualified_activation.clone(),
        fixture.next_incarnation.clone(),
        fixture.next_public_head.clone(),
        provider_client,
    )
    .await
    .expect("open the promoted successor authority");
    let retained_after_promotion = successor_authority
        .read_status(&state_input, &status_request)
        .await;
    assert!(
        matches!(
            retained_after_promotion,
            ReadAdapterCompletion::Returned(WalletNonceStatus::Completed {
                completion: ref retained_completion,
                resource_head_ref: ref retained_head,
                ..
            }) if retained_completion == &completion
                && retained_head == &fixture.next_lineage_head.public_lineage_head_ref
        ),
        "unexpected retained status after promotion recovery: {retained_after_promotion:?}"
    );
    assert!(matches!(
        successor_authority
            .reserve(&fixture.refreshed_state_input(), &retry)
            .await,
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved {
            reservation: ref retained,
        }) if retained == &reservation
    ));

    let retained_in_flight_after_promotion = successor_authority
        .read_status(&state_input, &held_resolution_status_request)
        .await;
    assert!(
        matches!(
            retained_in_flight_after_promotion,
            ReadAdapterCompletion::Returned(WalletNonceStatus::Reserved {
                reservation: ref retained_reservation,
                resource_head_ref: ref retained_head,
                ..
            }) if retained_reservation == &held_resolution_reservation
                && retained_head == &fixture.next_lineage_head.public_lineage_head_ref
        ),
        "in-flight reservation did not survive physical refresh: {retained_in_flight_after_promotion:?}"
    );
    let refreshed_request = held_resolution_request;
    let refreshed_reservation = held_resolution_reservation;
    assert_eq!(refreshed_reservation.nonce, 10);
    let (refreshed_activation_request, _) =
        fixture.activation_request(&refreshed_request, &refreshed_reservation);
    let refreshed_candidate = match successor_authority
        .activate_candidate(&state_input, &refreshed_activation_request)
        .await
    {
        EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated { candidate }) => {
            candidate
        }
        other => panic!("unexpected refreshed activation result: {other:?}"),
    };
    let refreshed_completion_request =
        fixture.completion_request(&refreshed_reservation, &refreshed_candidate);
    let refreshed_completion = match successor_authority
        .complete(&state_input, &refreshed_completion_request)
        .await
    {
        EffectAdapterCompletion::Returned(CompleteWalletNonceResponse::Completed {
            completion,
        }) => completion,
        other => panic!("unexpected refreshed completion result: {other:?}"),
    };
    assert!(matches!(
        successor_authority
            .read_status(&state_input, &held_resolution_status_request)
            .await,
        ReadAdapterCompletion::Returned(WalletNonceStatus::Completed {
            completion: ref retained_completion,
            resource_head_ref: ref retained_head,
            ..
        }) if retained_completion == &refreshed_completion
            && retained_head == &fixture.next_lineage_head.public_lineage_head_ref
    ));

    let successor_request =
        fixture.reserve_request(qualified_activation.clone(), 11, "intent-successor");
    let successor_reservation = match successor_authority
        .reserve(&state_input, &successor_request)
        .await
    {
        EffectAdapterCompletion::Returned(ReserveWalletNonceResponse::Reserved { reservation }) => {
            reservation
        }
        other => panic!("unexpected successor reservation result: {other:?}"),
    };
    assert_eq!(successor_reservation.nonce, 11);
    let (successor_activation_request, _) =
        fixture.activation_request(&successor_request, &successor_reservation);
    let successor_candidate = match successor_authority
        .activate_candidate(&state_input, &successor_activation_request)
        .await
    {
        EffectAdapterCompletion::Returned(ActivateCandidateResponse::Activated { candidate }) => {
            candidate
        }
        other => panic!("unexpected successor activation result: {other:?}"),
    };
    let successor_completion_request =
        fixture.completion_request(&successor_reservation, &successor_candidate);
    let successor_completion = match successor_authority
        .complete(&state_input, &successor_completion_request)
        .await
    {
        EffectAdapterCompletion::Returned(CompleteWalletNonceResponse::Completed {
            completion,
        }) => completion,
        other => panic!("unexpected successor completion result: {other:?}"),
    };
    let successor_status_request = ReadEvmWalletNonceStatusRequest {
        nonce_domain: successor_request.nonce_domain.clone(),
        domain_activation_attestation: qualified_activation.clone(),
        semantic_reservation_key: successor_request.reservation_key.clone(),
        submission_intent_id: successor_request.submission_intent_id.clone(),
        transaction_intent_digest: successor_request.transaction_intent.digest().to_owned(),
        candidate_family_ref: successor_request.candidate_family.digest().to_owned(),
    };
    assert!(matches!(
        successor_authority
            .read_status(&state_input, &successor_status_request)
            .await,
        ReadAdapterCompletion::Returned(WalletNonceStatus::Completed {
            completion: ref retained_completion,
            resource_head_ref: ref retained_head,
            ..
        }) if retained_completion == &successor_completion
                && retained_head == &fixture.next_lineage_head.public_lineage_head_ref
    ));
    for forged_high_water in [Some(10), Some(12), None] {
        assert_high_water_rewrite_rejected(
            &mut admin_connection,
            &schema,
            successor_request.nonce_domain.as_str(),
            11,
            forged_high_water,
            &successor_authority,
            &state_input,
            &successor_status_request,
        )
        .await;
    }
    assert_omitted_reservation_rejected(
        &mut admin_connection,
        &schema,
        reservation.semantic_reservation_key.as_str(),
        &successor_authority,
        &state_input,
        &successor_status_request,
    )
    .await;
    assert_status_scalar_rewrite_rejected(
        &mut admin_connection,
        &schema,
        "wallet_nonce_domains",
        "wallet_nonce_domain_id",
        "wallet_nonce_domain_id",
        successor_request.nonce_domain.as_str(),
        "mfm.evm.test/torn-wallet-nonce-domain-id",
        "mfm.evm.test/torn-wallet-nonce-domain-id",
        &successor_authority,
        &state_input,
        &successor_status_request,
    )
    .await;
    assert_status_scalar_rewrite_rejected(
        &mut admin_connection,
        &schema,
        "wallet_nonce_domains",
        "activation_record_ref",
        "wallet_nonce_domain_id",
        successor_request.nonce_domain.as_str(),
        "mfm.evm.test/torn-activation-record-ref",
        successor_request.nonce_domain.as_str(),
        &successor_authority,
        &state_input,
        &successor_status_request,
    )
    .await;
    assert_status_scalar_rewrite_rejected(
        &mut admin_connection,
        &schema,
        "wallet_nonce_reservations",
        "submission_intent_id",
        "semantic_reservation_key",
        successor_reservation.semantic_reservation_key.as_str(),
        "mfm.evm.test/torn-submission-intent",
        successor_reservation.semantic_reservation_key.as_str(),
        &successor_authority,
        &state_input,
        &successor_status_request,
    )
    .await;
    assert_status_scalar_rewrite_rejected(
        &mut admin_connection,
        &schema,
        "wallet_nonce_completions",
        "semantic_completion_key",
        "semantic_completion_key",
        successor_completion.semantic_completion_key.as_str(),
        "mfm.evm.test/torn-completion-key",
        "mfm.evm.test/torn-completion-key",
        &successor_authority,
        &state_input,
        &successor_status_request,
    )
    .await;
    let original_completion_json = corrupt_completion_semantically(
        &mut admin_connection,
        &schema,
        successor_completion.semantic_completion_key.as_str(),
    )
    .await;
    assert!(matches!(
        successor_authority
            .read_status(&state_input, &successor_status_request)
            .await,
        ReadAdapterCompletion::IntegrityFault(_)
    ));
    let allocation_after_forged_completion = fixture.reserve_request(
        qualified_activation.clone(),
        11,
        "allocation-after-forged-completion",
    );
    assert!(matches!(
        successor_authority
            .reserve(&state_input, &allocation_after_forged_completion)
            .await,
        EffectAdapterCompletion::IntegrityFault(_)
    ));
    assert_reservation_absent(&probe_pool, &allocation_after_forged_completion).await;
    assert_no_provider_wire_evidence(
        &probe_pool,
        &provider_public_key,
        &provider.endpoint().to_string_lossy(),
    )
    .await;
    restore_completion_semantically(
        &mut admin_connection,
        &schema,
        successor_completion.semantic_completion_key.as_str(),
        original_completion_json,
    )
    .await;
    assert!(matches!(
        successor_authority
            .read_status(&state_input, &successor_status_request)
            .await,
        ReadAdapterCompletion::Returned(WalletNonceStatus::Completed {
            completion: ref retained_completion,
            ..
        }) if retained_completion == &successor_completion
    ));
    let original_candidate_json = corrupt_candidate_prefix_semantically(
        &mut admin_connection,
        &schema,
        replacement_request.candidate_operation_key.as_str(),
    )
    .await;
    assert!(matches!(
        successor_authority
            .read_status(&state_input, &status_request)
            .await,
        ReadAdapterCompletion::IntegrityFault(_)
    ));
    restore_candidate_prefix_semantically(
        &mut admin_connection,
        &schema,
        replacement_request.candidate_operation_key.as_str(),
        original_candidate_json,
    )
    .await;
    assert!(matches!(
        successor_authority
            .read_status(&state_input, &status_request)
            .await,
        ReadAdapterCompletion::Returned(WalletNonceStatus::Completed {
            completion: ref retained_completion,
            ..
        }) if retained_completion == &completion
    ));

    drop(probe_pool);
    drop(successor_authority);
    drop(sibling_database_authority);
    drop(sibling_schema_authority);
    drop(ambiguity_authority);
    drop(commit_proxy);
    drop(authority_b);
    drop(authority_a);
    drop(public);
    drop(ambiguity_registry);
    drop(registry_b);
    drop(registry);
    let provider_endpoint = provider.endpoint().to_path_buf();
    drop(provider);
    let absent_provider_trust = WalletAuthorityProviderTrust::new(
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        &provider_public_key,
    )
    .expect("reconstruct public provider trust anchor");
    assert!(matches!(
        WalletAuthorityProviderClient::connect_unix(provider_endpoint, absent_provider_trust).await,
        Err(PostgresEvmWalletError::Unavailable)
    ));

    let successor_provider_config = ProviderProcessConfig::new(
        std::env::temp_dir().join(format!("{schema}-2.sock")),
        &checkpoint_authority,
        database_url_with_active_role(&role_urls.activation_admin, ACTIVATION_ADMIN_ROLE),
        database_url_with_active_role(
            &role_urls.nonce_application,
            "mfm_evm_wallet_nonce_application",
        ),
        schema.clone(),
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        fixture
            .refreshed_routing_catalog
            .chain_registry_head_ref()
            .clone(),
        fixture.registry_lineage_ref.clone(),
        fixture.next_provider_fence_head_ref.clone(),
        fixture.next_incarnation.clone(),
        fixture.next_public_head.clone(),
        vec![
            fixture.routing_catalog.clone(),
            fixture.refreshed_routing_catalog.clone(),
        ],
        vec![
            fixture.activation_record.clone(),
            fixture.secondary_activation_record.clone(),
            fault_record,
            competing_record_a,
            competing_record_b,
            sibling_record_a,
            sibling_record_b,
        ],
        Vec::new(),
        None,
    )
    .expect("configure qualified provider-key successor")
    .with_allowed_issuance_incarnations(vec![
        fixture.next_incarnation.clone(),
        fixture.incarnation.clone(),
        fault_incarnation,
        competing_incarnation_a,
        competing_incarnation_b,
        sibling_incarnation_a,
        sibling_incarnation_b,
    ])
    .expect("retain every immutable activation incarnation after provider-key succession");
    let successor_provider = ProviderProcess::spawn(
        env!("CARGO_BIN_EXE_mfm-storage-wallet-authority-provider"),
        successor_provider_config,
    )
    .expect("start qualified provider-key successor");
    let successor_provider_public_key = successor_provider.public_key_hex().to_owned();
    assert_ne!(successor_provider_public_key, provider_public_key);
    let stale_key_trust = WalletAuthorityProviderTrust::new(
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        fixture
            .refreshed_routing_catalog
            .chain_registry_head_ref()
            .clone(),
        &provider_public_key,
    )
    .expect("construct stale provider-key trust");
    assert!(matches!(
        WalletAuthorityProviderClient::connect_unix(
            successor_provider.endpoint(),
            stale_key_trust,
        )
        .await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));
    let successor_key_trust = WalletAuthorityProviderTrust::new(
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        fixture
            .refreshed_routing_catalog
            .chain_registry_head_ref()
            .clone(),
        &successor_provider_public_key,
    )
    .expect("construct successor provider-key trust");
    let successor_provider_client = WalletAuthorityProviderClient::connect_unix(
        successor_provider.endpoint(),
        successor_key_trust,
    )
    .await
    .expect("authenticate qualified provider-key successor");
    assert_eq!(
        successor_provider_client
            .qualify_routing_catalog(&fixture.refreshed_routing_catalog)
            .await
            .expect("qualify append-only catalog refresh after provider restart")
            .descriptor(),
        &fixture.refreshed_routing_catalog
    );
    assert!(fixture
        .refreshed_routing_catalog
        .generations()
        .iter()
        .all(|generation| generation.chain_instance()
            == &fixture.chain_attestation.binding().expect("chain binding")));
    assert!(matches!(
        successor_provider_client
            .qualify_routing_catalog(&fixture.routing_catalog)
            .await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));
    assert_unissued_chain_catalogs_fail_closed(
        &successor_provider_client,
        &fixture,
        &fixture.refreshed_routing_catalog,
    )
    .await;
    assert_eq!(
        request.reservation_key,
        fixture
            .reserve_request(qualified_activation, 7, "intent-one")
            .reservation_key,
        "provider and store succession must not change permanent semantic keys"
    );
    drop(successor_provider_client);
    drop(successor_provider);
    sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&mut admin_connection)
        .await
        .expect("drop isolated wallet schema");
    sqlx::query(AssertSqlSafe(format!(
        "DROP TABLE public.{public_sentinel}"
    )))
    .execute(&mut admin_connection)
    .await
    .expect("drop public-schema sentinel");
    sqlx::query(AssertSqlSafe(format!(
        "DROP SCHEMA {sibling_schema} CASCADE"
    )))
    .execute(&mut admin_connection)
    .await
    .expect("drop sibling wallet schema");
    drop(sibling_database_connection);
    sqlx::query(AssertSqlSafe(format!("DROP DATABASE {sibling_database}")))
        .execute(&mut admin_connection)
        .await
        .expect("drop sibling wallet database");
}

#[tokio::test]
async fn authorized_promotion_serializes_against_activation_issuance() {
    let _test_guard = MANAGED_WALLET_TEST_LOCK.lock().await;
    let base_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL is required for parity tests");
    let schema = unique_schema();
    let admin_options = PgConnectOptions::from_str(&base_url).expect("parse DATABASE_URL");
    let mut admin_connection = PgConnection::connect_with(&admin_options)
        .await
        .expect("connect PostgreSQL administrator");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&mut admin_connection)
        .await
        .expect("create promotion-race schema");
    let database_url = scoped_database_url(&base_url, &schema);
    PostgresEvmWalletSchema::migrate(&database_url)
        .await
        .expect("migrate promotion-race authority");
    let role_urls = WalletRoleDatabaseUrls::for_schema(&schema);
    configure_wallet_login_principals(&mut admin_connection, &role_urls).await;

    let fixture = Fixture::new();
    let lineage_id = format!("mfm.evm.test/{schema}-promotion-race-lineage");
    let current_incarnation = initial_incarnation(
        &fixture,
        &lineage_id,
        &format!("mfm.evm.test/{schema}-promotion-race-current"),
    );
    let mut next_incarnation = current_incarnation.clone();
    next_incarnation.writer_epoch = 2;
    next_incarnation.physical_target_instance_id =
        format!("mfm.evm.test/{schema}-promotion-race-next");
    let initial_record = activation_record_for_sender(&fixture, 0x61, &current_incarnation);
    let racing_record = activation_record_for_sender(&fixture, 0x62, &current_incarnation);
    let successor = WalletNonceStoreSuccessor {
        wallet_nonce_store_lineage_id: lineage_id.clone(),
        expected_current_incarnation_ref: canonical_wallet_reference(&current_incarnation)
            .expect("promotion-race current incarnation reference"),
        next_incarnation: next_incarnation.clone(),
    };
    let current_public_head = history_object(&format!("{schema}-promotion-race-current"), 1);
    let next_public_head = history_object(&format!("{schema}-promotion-race-next"), 2);
    let current_provider_fence_head_ref =
        EvmWalletReference::from_content_ref(current_public_head.content_ref.clone());
    let next_provider_fence_head_ref =
        EvmWalletReference::from_content_ref(next_public_head.content_ref.clone());
    let checkpoint_authority = ProviderCheckpointAuthority::start(
        &database_url_with_active_role(&role_urls.activation_admin, ACTIVATION_ADMIN_ROLE),
        &database_url_with_active_role(
            &role_urls.nonce_application,
            "mfm_evm_wallet_nonce_application",
        ),
        &schema,
        fixture.provider_fence_lineage_ref.clone(),
        current_incarnation.clone(),
        current_public_head.clone(),
        current_provider_fence_head_ref.clone(),
    )
    .await
    .expect("start promotion-race checkpoint authority");
    let provider_config = ProviderProcessConfig::new(
        std::env::temp_dir().join(format!("{schema}.sock")),
        &checkpoint_authority,
        database_url_with_active_role(&role_urls.activation_admin, ACTIVATION_ADMIN_ROLE),
        database_url_with_active_role(
            &role_urls.nonce_application,
            "mfm_evm_wallet_nonce_application",
        ),
        schema.clone(),
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        fixture.registry_lineage_ref.clone(),
        current_provider_fence_head_ref,
        current_incarnation.clone(),
        current_public_head,
        vec![fixture.routing_catalog.clone()],
        vec![initial_record.clone(), racing_record.clone()],
        vec![AllowedPromotion {
            successor: successor.clone(),
            next_public_lineage_head: next_public_head,
            next_provider_fence_head_ref,
        }],
        None,
    )
    .expect("configure promotion-race provider");
    let mut provider = ProviderProcess::spawn(
        env!("CARGO_BIN_EXE_mfm-storage-wallet-authority-provider"),
        provider_config,
    )
    .expect("start promotion-race provider");
    let trust = WalletAuthorityProviderTrust::new(
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        provider.public_key_hex(),
    )
    .expect("construct promotion-race provider trust");
    let client = WalletAuthorityProviderClient::connect_unix(provider.endpoint(), trust)
        .await
        .expect("authenticate promotion-race provider");
    let registry = open_activation_registry_admin(&role_urls.activation_admin, &schema, client)
        .await
        .expect("open promotion-race registry");
    registry
        .issue_domain_activation(&initial_record, &current_incarnation)
        .await
        .expect("establish promotion-race lineage");
    provider
        .revoke()
        .expect("fence promotion-race provider target");
    let _captured_prefix_digest = provider
        .authorize_promotion()
        .expect("authorize promotion-race successor");

    let mut blocker = PgConnection::connect_with(&admin_options)
        .await
        .expect("open promotion-race advisory blocker");
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(&lineage_id)
        .execute(&mut blocker)
        .await
        .expect("hold promotion-race lineage lock");
    let (class_id, object_id, object_sub_id) = sqlx::query_as::<_, (String, String, i32)>(
        "SELECT classid::text, objid::text, objsubid::integer \
             FROM pg_locks WHERE pid = pg_backend_pid() \
               AND locktype = 'advisory' AND granted",
    )
    .fetch_one(&mut blocker)
    .await
    .expect("identify held promotion-race advisory lock");
    let release = async {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let waiters = sqlx::query_scalar::<_, i64>(
                    "SELECT count(*)::bigint FROM pg_locks \
                     WHERE locktype = 'advisory' AND NOT granted \
                       AND classid::text = $1 AND objid::text = $2 \
                       AND objsubid::integer = $3",
                )
                .bind(&class_id)
                .bind(&object_id)
                .bind(object_sub_id)
                .fetch_one(&mut admin_connection)
                .await
                .expect("count promotion-race lock waiters");
                if waiters == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("both issuance and promotion reach the shared lineage lock");
        assert!(sqlx::query_scalar::<_, bool>(
            "SELECT pg_advisory_unlock(hashtextextended($1, 0))"
        )
        .bind(&lineage_id)
        .fetch_one(&mut blocker)
        .await
        .expect("release promotion-race lineage lock"));
    };
    let (issuance, promotion, ()) = tokio::join!(
        registry.issue_domain_activation(&racing_record, &current_incarnation),
        registry.promote_store_incarnation(&successor),
        release,
    );
    assert!(
        matches!(
            &issuance,
            Err(PostgresEvmWalletError::FenceRejected | PostgresEvmWalletError::PermanentConflict)
        ),
        "unexpected raced issuance result: {issuance:?}"
    );
    let promotion = promotion.expect("authorized promotion wins the shared-lineage race");
    assert_eq!(promotion.wallet_nonce_store_lineage_id, lineage_id);
    assert_eq!(promotion.writer_epoch, 2);
    assert_eq!(
        promotion.current_incarnation_ref,
        canonical_wallet_reference(&next_incarnation).expect("next incarnation reference")
    );
    let public = open_activation_registry_public(&role_urls.activation_public, &schema)
        .await
        .expect("open promotion-race public registry");
    assert_eq!(
        public
            .current_store_incarnation(&lineage_id)
            .await
            .expect("resolve promoted race winner"),
        Some(next_incarnation)
    );
    assert_eq!(
        public
            .domain_activation(&racing_record.wallet_nonce_domain)
            .await
            .expect("resolve losing raced activation"),
        None
    );

    drop(public);
    drop(registry);
    drop(provider);
    drop(blocker);
    sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&mut admin_connection)
        .await
        .expect("drop promotion-race schema");
}

#[allow(clippy::too_many_arguments)]
async fn assert_hostile_restore_prefix_rejected(
    admin: &mut PgConnection,
    schema: &str,
    registry: &mfm_storage_evm_postgres::PostgresWalletActivationRegistryAdmin,
    successor: &WalletNonceStoreSuccessor,
    fixture: &Fixture,
    activation: &WalletNonceDomainActivationAttestation,
    reservation: &ReservedWalletNonce,
    candidate: &ActiveWalletCandidate,
    completion: &CompletedWalletNonce,
) {
    let mut rewritten_incarnation = fixture.incarnation.clone();
    let original_target_key = fixture
        .incarnation
        .non_exportable_target_public_key_ref
        .to_content_ref()
        .expect("original target-key reference");
    rewritten_incarnation.non_exportable_target_public_key_ref =
        EvmWalletReference::from_content_ref(
            ContentRef::new(
                original_target_key.schema_id().clone(),
                ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    sha256_digest_bytes(b"hostile-restored-target-key"),
                ),
            )
            .expect("rewritten target-key content reference"),
        );
    assert_ne!(rewritten_incarnation, fixture.incarnation);
    assert_prefix_text_rewrite_rejected(
        admin,
        schema,
        "wallet_store_incarnations",
        "incarnation_json",
        "wallet_nonce_store_lineage_id",
        &fixture.incarnation.wallet_nonce_store_lineage_id,
        canonical_json(&rewritten_incarnation)
            .expect("canonical rewritten target-key record")
            .as_str(),
        registry,
        successor,
    )
    .await;

    let mut rewritten_record = fixture.activation_record.clone();
    rewritten_record.new_idempotency_epoch =
        stable("mfm.evm.test/hostile-restored-idempotency-epoch")
            .as_str()
            .to_owned();
    rewritten_record
        .validate()
        .expect("well-formed rewritten domain-activation record");
    assert_prefix_text_rewrite_rejected(
        admin,
        schema,
        "wallet_domain_activations",
        "activation_record_json",
        "wallet_nonce_domain_id",
        fixture.nonce_domain.as_str(),
        canonical_json(&rewritten_record)
            .expect("canonical rewritten activation record")
            .as_str(),
        registry,
        successor,
    )
    .await;

    let mut rewritten_activation = activation.clone();
    rewritten_activation.activation_record_ref =
        canonical_wallet_reference(&rewritten_record).expect("rewritten activation record ref");
    rewritten_activation.current_schema_record = rewritten_record;
    rewritten_activation
        .validate()
        .expect("well-formed rewritten local activation binding");
    assert_prefix_text_rewrite_rejected(
        admin,
        schema,
        "wallet_nonce_domains",
        "activation_attestation_json",
        "wallet_nonce_domain_id",
        fixture.nonce_domain.as_str(),
        canonical_json(&rewritten_activation)
            .expect("canonical rewritten activation binding")
            .as_str(),
        registry,
        successor,
    )
    .await;

    let mut rewritten_reservation = reservation.clone();
    rewritten_reservation.reservation_evidence_ref = fixture.next_provider_fence_head_ref.clone();
    assert_prefix_text_rewrite_rejected(
        admin,
        schema,
        "wallet_nonce_reservations",
        "reservation_json",
        "semantic_reservation_key",
        reservation.semantic_reservation_key.as_str(),
        canonical_json(&rewritten_reservation)
            .expect("canonical rewritten reservation result")
            .as_str(),
        registry,
        successor,
    )
    .await;

    let mut rewritten_candidate = candidate.clone();
    rewritten_candidate.attested_candidate.transaction_hash =
        format!("{:#x}", B256::repeat_byte(0x77));
    let candidate_key = derive_evm_candidate_operation_key(
        &candidate.attested_candidate.semantic_reservation_key,
        candidate.attested_candidate.candidate_ordinal,
    )
    .expect("candidate operation key");
    assert_prefix_text_rewrite_rejected(
        admin,
        schema,
        "wallet_nonce_candidates",
        "active_candidate_json",
        "semantic_candidate_operation_key",
        candidate_key.as_str(),
        canonical_json(&rewritten_candidate)
            .expect("canonical rewritten candidate member")
            .as_str(),
        registry,
        successor,
    )
    .await;

    assert_prefix_row_omission_rejected(
        admin,
        schema,
        "wallet_nonce_completions",
        "semantic_completion_key",
        completion.semantic_completion_key.as_str(),
        registry,
        successor,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn assert_prefix_text_rewrite_rejected(
    admin: &mut PgConnection,
    schema: &str,
    table: &str,
    column: &str,
    key_column: &str,
    key: &str,
    replacement: &str,
    registry: &mfm_storage_evm_postgres::PostgresWalletActivationRegistryAdmin,
    successor: &WalletNonceStoreSuccessor,
) {
    let table = qualified_test_table(schema, table);
    let column = test_identifier(column);
    let key_column = test_identifier(key_column);
    let original = sqlx::query_scalar::<_, String>(AssertSqlSafe(format!(
        "SELECT {column} FROM {table} WHERE {key_column} = $1"
    )))
    .bind(key)
    .fetch_one(&mut *admin)
    .await
    .expect("capture immutable prefix member before hostile rewrite");
    assert_ne!(original, replacement);

    set_replication_role(admin, "replica").await;
    let changed = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {table} SET {column} = $2 WHERE {key_column} = $1"
    )))
    .bind(key)
    .bind(replacement)
    .execute(&mut *admin)
    .await
    .expect("inject hostile immutable prefix rewrite");
    assert_eq!(changed.rows_affected(), 1);
    set_replication_role(admin, "origin").await;

    let result = registry.promote_store_incarnation(successor).await;

    set_replication_role(admin, "replica").await;
    let restored = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {table} SET {column} = $2 WHERE {key_column} = $1"
    )))
    .bind(key)
    .bind(original)
    .execute(&mut *admin)
    .await
    .expect("restore immutable prefix member after hostile rewrite");
    assert_eq!(restored.rows_affected(), 1);
    set_replication_role(admin, "origin").await;

    assert!(matches!(result, Err(PostgresEvmWalletError::FenceRejected)));
}

#[allow(clippy::too_many_arguments)]
async fn assert_prefix_row_omission_rejected(
    admin: &mut PgConnection,
    schema: &str,
    table: &str,
    key_column: &str,
    key: &str,
    registry: &mfm_storage_evm_postgres::PostgresWalletActivationRegistryAdmin,
    successor: &WalletNonceStoreSuccessor,
) {
    let table = qualified_test_table(schema, table);
    let key_column = test_identifier(key_column);
    let original = sqlx::query_scalar::<_, String>(AssertSqlSafe(format!(
        "SELECT to_jsonb(retained)::text FROM {table} AS retained WHERE {key_column} = $1"
    )))
    .bind(key)
    .fetch_one(&mut *admin)
    .await
    .expect("capture immutable prefix member before hostile omission");

    set_replication_role(admin, "replica").await;
    sqlx::query(AssertSqlSafe(format!(
        "DELETE FROM {table} WHERE {key_column} = $1"
    )))
    .bind(key)
    .execute(&mut *admin)
    .await
    .expect("inject hostile immutable prefix omission");
    set_replication_role(admin, "origin").await;

    let result = registry.promote_store_incarnation(successor).await;

    set_replication_role(admin, "replica").await;
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO {table} SELECT * FROM jsonb_populate_record(NULL::{table}, $1::jsonb)"
    )))
    .bind(original)
    .execute(&mut *admin)
    .await
    .expect("restore immutable prefix member after hostile omission");
    set_replication_role(admin, "origin").await;

    assert!(matches!(result, Err(PostgresEvmWalletError::FenceRejected)));
}

async fn set_replication_role(admin: &mut PgConnection, role: &str) {
    assert!(matches!(role, "replica" | "origin"));
    sqlx::query(AssertSqlSafe(format!(
        "SET session_replication_role = {role}"
    )))
    .execute(admin)
    .await
    .expect("set hostile-restore replication role");
}

fn qualified_test_table(schema: &str, table: &str) -> String {
    format!("{}.{}", test_identifier(schema), test_identifier(table))
}

fn test_identifier(value: &str) -> String {
    assert!(!value.is_empty());
    assert!(value.chars().enumerate().all(|(index, character)| {
        character == '_'
            || (character.is_ascii_alphanumeric() && (index > 0 || !character.is_ascii_digit()))
    }));
    format!("\"{value}\"")
}

fn assert_hostile_routing_history_restart_fails_closed(
    provider_config: &ProviderProcessConfig,
    fixture: &Fixture,
) {
    let current_head = fixture
        .refreshed_routing_catalog
        .chain_registry_head_ref()
        .clone();
    let hostile_attestation = ChainInstanceRegistryAttestation::new(
        ChainInstanceDeclaration::new(
            fixture
                .chain_attestation
                .declaration()
                .chain_registry_lineage_ref()
                .clone(),
            stable("mfm.evm.test/hostile-remapped-chain-instance"),
            1,
            B256::repeat_byte(0x71),
            U256::from(101_u64),
            B256::repeat_byte(0x72),
        )
        .expect("hostile remapped chain declaration"),
        authority_reference("chain-registry-issuance-hostile-remap"),
        current_head.clone(),
    )
    .expect("hostile remapped chain attestation");
    let hostile_route = EvmRoutingGenerationDescriptor::new(
        "wallet-test-network",
        "mfm.evm.json-rpc",
        stable("mfm.evm.test/hostile-remapped-route-generation"),
        authority_reference("route-membership-hostile-remap"),
        hostile_attestation
            .binding()
            .expect("hostile remapped chain binding"),
    )
    .expect("hostile remapped route");
    let mut chain_instances = fixture.routing_catalog.chain_instances().to_vec();
    chain_instances.push(hostile_attestation);
    let mut generations = fixture.routing_catalog.generations().to_vec();
    generations.push(hostile_route);
    let hostile_catalog =
        EvmRoutingCatalogDescriptor::new(current_head.clone(), chain_instances, generations)
            .expect("structurally valid hostile route remap");
    assert!(provider_config
        .clone()
        .with_routing_catalog_history(
            current_head,
            vec![fixture.routing_catalog.clone(), hostile_catalog],
        )
        .is_err());
}

async fn assert_unissued_chain_catalogs_fail_closed(
    client: &WalletAuthorityProviderClient,
    fixture: &Fixture,
    current_catalog: &EvmRoutingCatalogDescriptor,
) {
    let lineage = fixture
        .chain_attestation
        .declaration()
        .chain_registry_lineage_ref()
        .clone();
    let cases = [
        chain_attestation(
            lineage.clone(),
            "mfm.evm.test/unissued-clone",
            B256::repeat_byte(0x11),
            B256::repeat_byte(0x12),
            fixture,
        ),
        chain_attestation(
            lineage.clone(),
            "mfm.evm.test/unissued-genesis",
            B256::repeat_byte(0x13),
            B256::repeat_byte(0x12),
            fixture,
        ),
        chain_attestation(
            lineage,
            "mfm.evm.test/unissued-anchor",
            B256::repeat_byte(0x11),
            B256::repeat_byte(0x14),
            fixture,
        ),
        chain_attestation(
            fixture
                .chain_attestation
                .declaration()
                .chain_registry_lineage_ref()
                .clone(),
            "mfm.evm.test/chain-instance",
            B256::repeat_byte(0x11),
            B256::repeat_byte(0x15),
            fixture,
        ),
    ];
    for (ordinal, attestation) in cases.into_iter().enumerate() {
        let binding = attestation.binding().expect("unissued chain binding");
        assert_ne!(
            binding.qualified_chain_instance_id(),
            fixture
                .chain_attestation
                .binding()
                .expect("issued chain binding")
                .qualified_chain_instance_id(),
        );
        let catalog = EvmRoutingCatalogDescriptor::new(
            current_catalog.chain_registry_head_ref().clone(),
            vec![attestation],
            vec![EvmRoutingGenerationDescriptor::new(
                "wallet-test-network",
                format!("mfm.evm.unissued-route-{ordinal}"),
                stable(&format!("mfm.evm.test/unissued-generation-{ordinal}")),
                authority_reference(&format!("route-membership-unissued-{ordinal}")),
                binding,
            )
            .expect("unissued route")],
        )
        .expect("coherent unissued catalog");
        assert!(matches!(
            client.qualify_routing_catalog(&catalog).await,
            Err(PostgresEvmWalletError::FenceRejected)
        ));
    }

    let sibling_attestation = chain_attestation(
        evm_wallet_assurance_policy_ref().expect("sibling registry lineage"),
        "mfm.evm.test/sibling-registry-instance",
        B256::repeat_byte(0x11),
        B256::repeat_byte(0x12),
        fixture,
    );
    let sibling_binding = sibling_attestation
        .binding()
        .expect("sibling registry binding");
    let sibling_catalog = EvmRoutingCatalogDescriptor::new(
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        vec![sibling_attestation],
        vec![EvmRoutingGenerationDescriptor::new(
            "wallet-test-network",
            "mfm.evm.sibling-registry-route",
            stable("mfm.evm.test/sibling-registry-generation"),
            authority_reference("route-membership-sibling-registry"),
            sibling_binding,
        )
        .expect("sibling registry route")],
    )
    .expect("sibling registry catalog");
    assert!(matches!(
        client.qualify_routing_catalog(&sibling_catalog).await,
        Err(PostgresEvmWalletError::InvalidAuthority)
    ));

    let rolled_back_catalog = EvmRoutingCatalogDescriptor::new(
        authority_reference("chain-registry-head-rolled-back"),
        fixture.routing_catalog.chain_instances().to_vec(),
        fixture.routing_catalog.generations().to_vec(),
    )
    .expect("structurally coherent rolled-back-head catalog");
    assert!(matches!(
        client.qualify_routing_catalog(&rolled_back_catalog).await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));

    let foreign_route_catalog = EvmRoutingCatalogDescriptor::new(
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        vec![fixture.chain_attestation.clone()],
        vec![EvmRoutingGenerationDescriptor::new(
            "wallet-test-network",
            "mfm.evm.unissued-route",
            stable("mfm.evm.test/unissued-route-generation"),
            authority_reference("route-membership-unissued-foreign"),
            fixture
                .chain_attestation
                .binding()
                .expect("issued chain binding"),
        )
        .expect("unissued route descriptor")],
    )
    .expect("genuine attestation on unissued route catalog");
    assert!(matches!(
        client.qualify_routing_catalog(&foreign_route_catalog).await,
        Err(PostgresEvmWalletError::FenceRejected)
    ));
}

fn assert_activation_requires_issued_chain_and_exact_route(
    database_url: &str,
    nonce_database_url: &str,
    schema: &str,
    fixture: &Fixture,
    checkpoint_authority: &ProviderCheckpointAuthority,
) {
    let mut wrong_route = fixture.activation_record.clone();
    wrong_route.initial_route_generation_ref = fixture.redundant_route_generation_ref.clone();
    wrong_route
        .validate()
        .expect("wrong-route activation remains structurally coherent");

    let mut wrong_membership = fixture.activation_record.clone();
    wrong_membership.initial_route_membership_issuance_ref =
        fixture.redundant_route_membership_ref.clone();
    wrong_membership
        .validate()
        .expect("wrong-membership activation remains structurally coherent");

    let unissued_attestation = chain_attestation(
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        "mfm.evm.test/unissued-activation-instance",
        B256::repeat_byte(0x11),
        B256::repeat_byte(0x12),
        fixture,
    );
    let unissued_lineage = derive_evm_chain_lineage_id(
        unissued_attestation
            .binding()
            .expect("unissued activation binding")
            .qualified_chain_instance_id(),
    )
    .expect("unissued activation lineage");
    let mut absent_chain = fixture.activation_record.clone();
    absent_chain.chain_instance_attestation = unissued_attestation;
    absent_chain.wallet_nonce_domain =
        derive_wallet_nonce_domain(unissued_lineage, Address::repeat_byte(0x21))
            .expect("unissued activation domain");
    absent_chain
        .validate()
        .expect("absent-chain activation remains structurally coherent");

    for (case, record) in [
        ("wrong-route", wrong_route),
        ("wrong-membership", wrong_membership),
        ("absent-chain", absent_chain),
    ] {
        let config = ProviderProcessConfig::new(
            std::env::temp_dir().join(format!("{schema}-{case}.sock")),
            checkpoint_authority,
            database_url.to_owned(),
            nonce_database_url.to_owned(),
            schema.to_owned(),
            fixture.provider_id.clone(),
            fixture.provider_fence_lineage_ref.clone(),
            fixture
                .chain_attestation
                .declaration()
                .chain_registry_lineage_ref()
                .clone(),
            fixture.routing_catalog.chain_registry_head_ref().clone(),
            fixture.registry_lineage_ref.clone(),
            fixture.provider_fence_head_ref.clone(),
            fixture.incarnation.clone(),
            fixture.current_public_head.clone(),
            vec![fixture.routing_catalog.clone()],
            vec![record],
            Vec::new(),
            None,
        );
        assert!(config.is_err(), "{case} activation must not be issuable");
    }
}

fn chain_attestation(
    lineage: EvmWalletReference,
    namespace: &str,
    genesis: B256,
    anchor: B256,
    fixture: &Fixture,
) -> ChainInstanceRegistryAttestation {
    ChainInstanceRegistryAttestation::new(
        ChainInstanceDeclaration::new(
            lineage,
            stable(namespace),
            1,
            genesis,
            U256::from(100_u64),
            anchor,
        )
        .expect("chain declaration"),
        authority_reference(&format!("chain-registry-issuance-{namespace}")),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
    )
    .expect("chain attestation")
}

async fn prove_hostile_qualification_reopens(
    admin: &mut PgConnection,
    public_database_url: &str,
    schema: &str,
) {
    let schema = test_identifier(schema);
    let public_login = test_identifier(&database_login(public_database_url));
    for (grant, revoke, case) in [
        (
            format!("GRANT USAGE ON SCHEMA {schema} TO PUBLIC"),
            format!("REVOKE USAGE ON SCHEMA {schema} FROM PUBLIC"),
            "PUBLIC schema USAGE",
        ),
        (
            format!("GRANT CREATE ON SCHEMA {schema} TO PUBLIC"),
            format!("REVOKE CREATE ON SCHEMA {schema} FROM PUBLIC"),
            "PUBLIC schema CREATE",
        ),
        (
            format!(
                "GRANT UPDATE ON TABLE {schema}.wallet_nonce_domains \
                 TO mfm_evm_wallet_activation_public"
            ),
            format!(
                "REVOKE UPDATE ON TABLE {schema}.wallet_nonce_domains \
                 FROM mfm_evm_wallet_activation_public"
            ),
            "cross-surface table write",
        ),
        (
            format!(
                "GRANT EXECUTE ON FUNCTION {schema}.reject_immutable_wallet_row_change() TO PUBLIC"
            ),
            format!(
                "REVOKE EXECUTE ON FUNCTION {schema}.reject_immutable_wallet_row_change() FROM PUBLIC"
            ),
            "PUBLIC function EXECUTE",
        ),
        (
            "GRANT mfm_evm_wallet_owner TO mfm_evm_wallet_activation_public".to_owned(),
            "REVOKE mfm_evm_wallet_owner FROM mfm_evm_wallet_activation_public".to_owned(),
            "managed-role nesting grant",
        ),
        (
            format!(
                "GRANT mfm_evm_wallet_owner TO {public_login} \
                 WITH ADMIN FALSE, INHERIT FALSE, SET TRUE"
            ),
            format!("REVOKE mfm_evm_wallet_owner FROM {public_login}"),
            "incoming managed-role grant",
        ),
    ] {
        sqlx::query(AssertSqlSafe(grant))
            .execute(&mut *admin)
            .await
            .unwrap_or_else(|error| panic!("install hostile {case}: {error}"));
        assert!(
            matches!(
                open_activation_registry_public(public_database_url, schema.trim_matches('"'))
                    .await,
                Err(PostgresEvmWalletError::InvalidAuthority)
            ),
            "qualification accepted hostile {case}"
        );
        sqlx::query(AssertSqlSafe(revoke))
            .execute(&mut *admin)
            .await
            .unwrap_or_else(|error| panic!("restore hostile {case}: {error}"));
        drop(
            open_activation_registry_public(public_database_url, schema.trim_matches('"'))
                .await
                .unwrap_or_else(|error| panic!("restored {case} did not reopen: {error}")),
        );
    }
}

async fn prove_role_separation(
    database_url: &str,
    role_urls: &WalletRoleDatabaseUrls,
    schema: &str,
    public_sentinel: &str,
) {
    let options = PgConnectOptions::from_str(database_url)
        .expect("parse admin URL")
        .options([("search_path", schema)]);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open role-probe pool");
    sqlx::query("CREATE TABLE wallet_unlisted_future (value INTEGER NOT NULL)")
        .execute(&pool)
        .await
        .expect("create unlisted future wallet table");

    const ROLES: [&str; 6] = [
        "mfm_evm_wallet_owner",
        "mfm_evm_wallet_activation_admin",
        "mfm_evm_wallet_activation_public",
        "mfm_evm_wallet_nonce_application",
        "mfm_store_application",
        "mfm_evm_wallet_test",
    ];
    const TABLES: [&str; 8] = [
        "wallet_store_schema_metadata",
        "wallet_store_incarnations",
        "wallet_store_lineage_heads",
        "wallet_domain_activations",
        "wallet_nonce_domains",
        "wallet_nonce_reservations",
        "wallet_nonce_candidates",
        "wallet_nonce_completions",
    ];
    const PRIVILEGES: [&str; 7] = [
        "SELECT",
        "INSERT",
        "UPDATE",
        "DELETE",
        "TRUNCATE",
        "REFERENCES",
        "TRIGGER",
    ];

    for role in ROLES {
        for table in TABLES {
            for privilege in PRIVILEGES {
                let expected = expected_table_privilege(role, table, privilege);
                assert_eq!(
                    has_table_privilege(&pool, role, &format!("{schema}.{table}"), privilege).await,
                    expected,
                    "unexpected {privilege} privilege for {role} on {table}"
                );
            }
        }
        assert_eq!(
            has_schema_privilege(&pool, role, schema, "USAGE").await,
            role != "mfm_store_application",
            "unexpected schema USAGE for {role}"
        );
        assert_eq!(
            has_schema_privilege(&pool, role, schema, "CREATE").await,
            role == "mfm_evm_wallet_owner",
            "unexpected schema CREATE for {role}"
        );
        assert_eq!(
            has_function_privilege(
                &pool,
                role,
                &format!("{schema}.reject_immutable_wallet_row_change()"),
                "EXECUTE",
            )
            .await,
            role == "mfm_evm_wallet_owner",
            "unexpected trigger-function EXECUTE for {role}"
        );
        assert!(
            !has_table_privilege(&pool, role, &format!("public.{public_sentinel}"), "SELECT",)
                .await,
            "wallet role {role} reached the public-schema sentinel"
        );
        if role != "mfm_evm_wallet_owner" {
            assert!(
                !has_table_privilege(
                    &pool,
                    role,
                    &format!("{schema}.wallet_unlisted_future"),
                    "SELECT",
                )
                .await,
                "wallet role {role} inherited access to an unlisted future table"
            );
        }
    }

    assert_login_role_boundaries(
        &role_urls.activation_admin,
        "mfm_evm_wallet_activation_admin",
        "wallet_domain_activations",
        "wallet_nonce_domains",
    )
    .await;
    assert_login_role_boundaries(
        &role_urls.activation_public,
        "mfm_evm_wallet_activation_public",
        "wallet_store_schema_metadata",
        "wallet_nonce_domains",
    )
    .await;
    assert_login_role_boundaries(
        &role_urls.nonce_application,
        "mfm_evm_wallet_nonce_application",
        "wallet_nonce_domains",
        "wallet_store_lineage_heads",
    )
    .await;
}

async fn assert_login_role_boundaries(
    database_url: &str,
    expected_role: &str,
    readable_table: &str,
    forbidden_table: &str,
) {
    let options = PgConnectOptions::from_str(database_url).expect("parse wallet login URL");
    let expected_login = options.get_username().to_owned();
    let mut connection = PgConnection::connect_with(&options)
        .await
        .expect("connect distinct wallet login principal");
    assert_eq!(
        session_roles(&mut connection).await,
        (expected_login.clone(), expected_login.clone())
    );

    for forbidden_role in [
        "mfm_evm_wallet_owner",
        "mfm_evm_wallet_activation_admin",
        "mfm_evm_wallet_activation_public",
        "mfm_evm_wallet_nonce_application",
        "mfm_store_application",
        "mfm_evm_wallet_test",
    ] {
        if forbidden_role != expected_role {
            assert!(
                sqlx::query(AssertSqlSafe(format!(
                    "SET ROLE {}",
                    test_identifier(forbidden_role)
                )))
                .execute(&mut connection)
                .await
                .is_err(),
                "{expected_login} unexpectedly reached {forbidden_role}"
            );
        }
    }

    sqlx::query(AssertSqlSafe(format!(
        "SET ROLE {}",
        test_identifier(expected_role)
    )))
    .execute(&mut connection)
    .await
    .expect("activate the one granted wallet role");
    assert_eq!(
        session_roles(&mut connection).await,
        (expected_login.clone(), expected_role.to_owned())
    );
    sqlx::query(AssertSqlSafe(format!("SELECT * FROM {readable_table}")))
        .fetch_optional(&mut connection)
        .await
        .expect("the activated wallet role reads its own surface");
    assert!(
        sqlx::query(AssertSqlSafe(format!("SELECT * FROM {forbidden_table}")))
            .fetch_optional(&mut connection)
            .await
            .is_err(),
        "the activated wallet role reached another wallet surface"
    );
    assert!(
        sqlx::query("SET ROLE mfm_evm_wallet_owner")
            .execute(&mut connection)
            .await
            .is_err(),
        "the activated wallet role reached the wallet owner"
    );

    connection
        .execute("RESET ROLE")
        .await
        .expect("reset wallet role");
    assert_eq!(
        session_roles(&mut connection).await,
        (expected_login.clone(), expected_login)
    );
    assert!(
        sqlx::query(AssertSqlSafe(format!("SELECT * FROM {readable_table}")))
            .fetch_optional(&mut connection)
            .await
            .is_err(),
        "NOINHERIT login retained wallet table access after RESET ROLE"
    );
}

async fn session_roles(connection: &mut PgConnection) -> (String, String) {
    sqlx::query_as("SELECT session_user::text, current_user::text")
        .fetch_one(connection)
        .await
        .expect("inspect wallet session identities")
}

async fn prove_copied_target_rejected(
    authority: &dyn WalletNonceAuthority,
    state_input: &LexicalValueRef,
    status_request: &ReadEvmWalletNonceStatusRequest,
    reserve_request: &ReserveEvmNonceRequest,
    activation_request: &ActivateEvmCandidateRequest,
    completion_request: &CompleteEvmNonceRequest,
) {
    assert!(matches!(
        authority.read_status(state_input, status_request).await,
        ReadAdapterCompletion::IntegrityFault(_)
    ));
    assert!(matches!(
        authority.reserve(state_input, reserve_request).await,
        EffectAdapterCompletion::IntegrityFault(_)
    ));
    assert!(matches!(
        authority
            .activate_candidate(state_input, activation_request)
            .await,
        EffectAdapterCompletion::IntegrityFault(_)
    ));
    assert!(matches!(
        authority.complete(state_input, completion_request).await,
        EffectAdapterCompletion::IntegrityFault(_)
    ));
}

fn assert_one_activation_creation_winner(
    results: (
        Result<WalletNonceDomainActivationAttestation, PostgresEvmWalletError>,
        Result<WalletNonceDomainActivationAttestation, PostgresEvmWalletError>,
    ),
) {
    match results {
        (Ok(_), Err(PostgresEvmWalletError::PermanentConflict))
        | (Err(PostgresEvmWalletError::PermanentConflict), Ok(_)) => {}
        other => panic!("expected one activation winner and one permanent conflict: {other:?}"),
    }
}

async fn assert_activation_creation_absent(
    connection: &mut PgConnection,
    schema: &str,
    lineage_id: &str,
    domain_id: &str,
) {
    let count = sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!(
        "SELECT (SELECT count(*) FROM {schema}.wallet_store_incarnations \
                   WHERE wallet_nonce_store_lineage_id = $1) \
              + (SELECT count(*) FROM {schema}.wallet_store_lineage_heads \
                   WHERE wallet_nonce_store_lineage_id = $1) \
              + (SELECT count(*) FROM {schema}.wallet_domain_activations \
                   WHERE wallet_nonce_domain_id = $2)"
    )))
    .bind(lineage_id)
    .bind(domain_id)
    .fetch_one(&mut *connection)
    .await
    .expect("inspect interrupted activation creation");
    assert_eq!(
        count, 0,
        "interrupted issuance must roll back all three rows"
    );
}

async fn assert_single_activation_creation(
    connection: &mut PgConnection,
    schema: &str,
    domain_ids: &[&str],
    lineage_ids: &[&str],
) {
    let mut activation_count = 0;
    for domain_id in domain_ids {
        activation_count += sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!(
            "SELECT count(*) FROM {schema}.wallet_domain_activations \
             WHERE wallet_nonce_domain_id = $1"
        )))
        .bind(domain_id)
        .fetch_one(&mut *connection)
        .await
        .expect("count raced activation rows");
    }
    assert_eq!(activation_count, 1);
    let mut incarnation_count = 0;
    let mut head_count = 0;
    for lineage_id in lineage_ids {
        incarnation_count += sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!(
            "SELECT count(*) FROM {schema}.wallet_store_incarnations \
             WHERE wallet_nonce_store_lineage_id = $1"
        )))
        .bind(lineage_id)
        .fetch_one(&mut *connection)
        .await
        .expect("count raced incarnation rows");
        head_count += sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!(
            "SELECT count(*) FROM {schema}.wallet_store_lineage_heads \
             WHERE wallet_nonce_store_lineage_id = $1"
        )))
        .bind(lineage_id)
        .fetch_one(&mut *connection)
        .await
        .expect("count raced lineage-head rows");
    }
    assert_eq!(incarnation_count, 1);
    assert_eq!(head_count, 1);
}

#[allow(clippy::too_many_arguments)]
async fn assert_high_water_rewrite_rejected(
    connection: &mut PgConnection,
    schema: &str,
    domain_id: &str,
    retained_nonce: u64,
    forged_nonce: Option<u64>,
    authority: &PostgresWalletNonceAuthority,
    state_input: &LexicalValueRef,
    status_request: &ReadEvmWalletNonceStatusRequest,
) {
    set_replication_role(connection, "replica").await;
    let changed = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {schema}.wallet_nonce_domains \
         SET local_high_water_nonce = $2::numeric WHERE wallet_nonce_domain_id = $1"
    )))
    .bind(domain_id)
    .bind(forged_nonce.map(|nonce| nonce.to_string()))
    .execute(&mut *connection)
    .await
    .expect("inject forged wallet high water");
    assert_eq!(changed.rows_affected(), 1);
    set_replication_role(connection, "origin").await;

    let result = authority.read_status(state_input, status_request).await;

    set_replication_role(connection, "replica").await;
    let restored = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {schema}.wallet_nonce_domains \
         SET local_high_water_nonce = $2::numeric WHERE wallet_nonce_domain_id = $1"
    )))
    .bind(domain_id)
    .bind(retained_nonce.to_string())
    .execute(&mut *connection)
    .await
    .expect("restore wallet high water after hostile probe");
    assert_eq!(restored.rows_affected(), 1);
    set_replication_role(connection, "origin").await;

    assert!(matches!(result, ReadAdapterCompletion::IntegrityFault(_)));
}

#[allow(clippy::too_many_arguments)]
async fn assert_omitted_reservation_rejected(
    connection: &mut PgConnection,
    schema: &str,
    reservation_key: &str,
    authority: &PostgresWalletNonceAuthority,
    state_input: &LexicalValueRef,
    status_request: &ReadEvmWalletNonceStatusRequest,
) {
    let snapshot_table = "mfm_saved_wallet_nonce_reservation";
    sqlx::query(AssertSqlSafe(format!(
        "CREATE TEMPORARY TABLE {snapshot_table} AS \
         SELECT * FROM {schema}.wallet_nonce_reservations \
         WHERE semantic_reservation_key = $1"
    )))
    .bind(reservation_key)
    .execute(&mut *connection)
    .await
    .expect("snapshot one retained reservation for omission probe");
    set_replication_role(connection, "replica").await;
    let deleted = sqlx::query(AssertSqlSafe(format!(
        "DELETE FROM {schema}.wallet_nonce_reservations \
         WHERE semantic_reservation_key = $1"
    )))
    .bind(reservation_key)
    .execute(&mut *connection)
    .await
    .expect("omit one middle reservation from the retained prefix");
    assert_eq!(deleted.rows_affected(), 1);
    set_replication_role(connection, "origin").await;

    let result = authority.read_status(state_input, status_request).await;

    set_replication_role(connection, "replica").await;
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO {schema}.wallet_nonce_reservations \
         SELECT * FROM {snapshot_table}"
    )))
    .execute(&mut *connection)
    .await
    .expect("restore omitted reservation after hostile probe");
    set_replication_role(connection, "origin").await;
    sqlx::query(AssertSqlSafe(format!("DROP TABLE {snapshot_table}")))
        .execute(&mut *connection)
        .await
        .expect("drop reservation omission snapshot");

    assert!(matches!(result, ReadAdapterCompletion::IntegrityFault(_)));
}

async fn prove_high_water_cannot_reset(database_url: &str, domain_id: &str, retained_nonce: u64) {
    let options = PgConnectOptions::from_str(database_url).expect("parse nonce login URL");
    let mut connection = PgConnection::connect_with(&options)
        .await
        .expect("connect nonce high-water probe");
    connection
        .execute("SET ROLE mfm_evm_wallet_nonce_application")
        .await
        .expect("select nonce application role");
    assert!(
        sqlx::query(
            "UPDATE wallet_nonce_domains SET local_high_water_nonce = NULL \
             WHERE wallet_nonce_domain_id = $1",
        )
        .bind(domain_id)
        .execute(&mut connection)
        .await
        .is_err(),
        "nonce application role must not clear retained high water"
    );
    assert!(
        sqlx::query(
            "UPDATE wallet_nonce_domains SET local_high_water_nonce = $2::numeric \
             WHERE wallet_nonce_domain_id = $1",
        )
        .bind(domain_id)
        .bind(retained_nonce.to_string())
        .execute(&mut connection)
        .await
        .is_err(),
        "nonce application role must not retain an equal high water through UPDATE"
    );
    assert!(
        sqlx::query(
            "UPDATE wallet_nonce_domains SET local_high_water_nonce = $2::numeric \
             WHERE wallet_nonce_domain_id = $1",
        )
        .bind(domain_id)
        .bind(retained_nonce.saturating_sub(1).to_string())
        .execute(&mut connection)
        .await
        .is_err(),
        "nonce application role must not decrease retained high water"
    );
    let retained = sqlx::query_scalar::<_, String>(
        "SELECT local_high_water_nonce::text FROM wallet_nonce_domains \
         WHERE wallet_nonce_domain_id = $1",
    )
    .bind(domain_id)
    .fetch_one(&mut connection)
    .await
    .expect("read retained high water after rejected resets");
    assert_eq!(retained, retained_nonce.to_string());
    connection.execute("RESET ROLE").await.expect("reset role");
}

#[allow(clippy::too_many_arguments)]
async fn assert_status_scalar_rewrite_rejected(
    connection: &mut PgConnection,
    schema: &str,
    table: &str,
    column: &str,
    key_column: &str,
    key: &str,
    replacement: &str,
    restoration_key: &str,
    authority: &PostgresWalletNonceAuthority,
    state_input: &LexicalValueRef,
    status_request: &ReadEvmWalletNonceStatusRequest,
) {
    let table = qualified_test_table(schema, table);
    let column = test_identifier(column);
    let key_column = test_identifier(key_column);
    let original = sqlx::query_scalar::<_, String>(AssertSqlSafe(format!(
        "SELECT {column} FROM {table} WHERE {key_column} = $1"
    )))
    .bind(key)
    .fetch_one(&mut *connection)
    .await
    .expect("capture authoritative scalar before corruption");
    assert_ne!(original, replacement);

    set_replication_role(connection, "replica").await;
    let changed = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {table} SET {column} = $2 WHERE {key_column} = $1"
    )))
    .bind(key)
    .bind(replacement)
    .execute(&mut *connection)
    .await
    .expect("inject scalar-only wallet row corruption");
    assert_eq!(changed.rows_affected(), 1);
    set_replication_role(connection, "origin").await;

    let result = authority.read_status(state_input, status_request).await;

    set_replication_role(connection, "replica").await;
    let restored = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {table} SET {column} = $2 WHERE {key_column} = $1"
    )))
    .bind(restoration_key)
    .bind(original)
    .execute(&mut *connection)
    .await
    .expect("restore authoritative scalar after corruption probe");
    assert_eq!(restored.rows_affected(), 1);
    set_replication_role(connection, "origin").await;

    assert!(matches!(result, ReadAdapterCompletion::IntegrityFault(_)));
}

async fn corrupt_candidate_prefix_semantically(
    connection: &mut PgConnection,
    schema: &str,
    candidate_operation_key: &str,
) -> (String, String) {
    let (original_request_json, original_candidate_json) =
        sqlx::query_as::<_, (String, String)>(AssertSqlSafe(format!(
            "SELECT request_json, active_candidate_json FROM {schema}.wallet_nonce_candidates \
             WHERE semantic_candidate_operation_key = $1"
        )))
        .bind(candidate_operation_key)
        .fetch_one(&mut *connection)
        .await
        .expect("load candidate prefix for semantic corruption probe");
    let mut request: serde_json::Value =
        serde_json::from_str(&original_request_json).expect("candidate request JSON");
    let mut candidate: serde_json::Value =
        serde_json::from_str(&original_candidate_json).expect("active candidate JSON");
    let forged_hash = format!("0x{}", "ab".repeat(32));
    request["next_candidate"]["transaction_hash"] = forged_hash.clone().into();
    candidate["attested_candidate"]["transaction_hash"] = forged_hash.into();
    let forged_request_json =
        serde_json::to_string(&request).expect("canonical candidate request JSON");
    let forged_candidate_json =
        serde_json::to_string(&candidate).expect("canonical active candidate JSON");
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {schema}.wallet_nonce_candidates DISABLE TRIGGER USER"
    )))
    .execute(&mut *connection)
    .await
    .expect("disable immutable-row trigger for corruption probe");
    let update = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {schema}.wallet_nonce_candidates \
         SET request_json = $2, active_candidate_json = $3 \
         WHERE semantic_candidate_operation_key = $1"
    )))
    .bind(candidate_operation_key)
    .bind(forged_request_json)
    .bind(forged_candidate_json)
    .execute(&mut *connection)
    .await;
    let reenable = sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {schema}.wallet_nonce_candidates ENABLE TRIGGER USER"
    )))
    .execute(&mut *connection)
    .await;
    reenable.expect("restore immutable-row trigger after corruption probe");
    assert_eq!(
        update
            .expect("inject structurally valid forged candidate prefix")
            .rows_affected(),
        1
    );
    (original_request_json, original_candidate_json)
}

async fn corrupt_completion_semantically(
    connection: &mut PgConnection,
    schema: &str,
    completion_key: &str,
) -> (String, String) {
    let (original_request_json, original_completion_json) =
        sqlx::query_as::<_, (String, String)>(AssertSqlSafe(format!(
            "SELECT request_json, completion_json FROM {schema}.wallet_nonce_completions \
             WHERE semantic_completion_key = $1"
        )))
        .bind(completion_key)
        .fetch_one(&mut *connection)
        .await
        .expect("load completion for semantic corruption probe");
    let mut request: serde_json::Value =
        serde_json::from_str(&original_request_json).expect("completion request JSON");
    let mut completion: serde_json::Value =
        serde_json::from_str(&original_completion_json).expect("completion JSON");
    request["terminal_witnesses"]["finalized_head"]["block_number"] = "103".into();
    request["terminal_witnesses"]["finalized_head"]["block_hash"] =
        format!("{:#x}", B256::repeat_byte(0xa3)).into();
    let witnesses: TerminalWitnesses =
        serde_json::from_value(request["terminal_witnesses"].clone())
            .expect("decode forged terminal witness closure");
    completion["original_terminal_witnesses_ref"] = canonical_wallet_reference(&witnesses)
        .expect("reference forged terminal witness closure")
        .content_digest()
        .into();
    let forged_request_json =
        serde_json::to_string(&request).expect("canonical completion request JSON");
    let forged_completion_json =
        serde_json::to_string(&completion).expect("canonical completion JSON");
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {schema}.wallet_nonce_completions DISABLE TRIGGER USER"
    )))
    .execute(&mut *connection)
    .await
    .expect("disable completion immutable-row trigger for corruption probe");
    let update = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {schema}.wallet_nonce_completions \
         SET request_json = $2, completion_json = $3 \
         WHERE semantic_completion_key = $1"
    )))
    .bind(completion_key)
    .bind(forged_request_json)
    .bind(forged_completion_json)
    .execute(&mut *connection)
    .await;
    let reenable = sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {schema}.wallet_nonce_completions ENABLE TRIGGER USER"
    )))
    .execute(&mut *connection)
    .await;
    reenable.expect("restore completion immutable-row trigger after corruption probe");
    assert_eq!(
        update
            .expect("inject structurally valid forged completion witnesses")
            .rows_affected(),
        1
    );
    (original_request_json, original_completion_json)
}

async fn restore_candidate_prefix_semantically(
    connection: &mut PgConnection,
    schema: &str,
    candidate_operation_key: &str,
    (request_json, candidate_json): (String, String),
) {
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {schema}.wallet_nonce_candidates DISABLE TRIGGER USER"
    )))
    .execute(&mut *connection)
    .await
    .expect("disable candidate trigger for exact corruption restoration");
    let update = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {schema}.wallet_nonce_candidates \
         SET request_json = $2, active_candidate_json = $3 \
         WHERE semantic_candidate_operation_key = $1"
    )))
    .bind(candidate_operation_key)
    .bind(request_json)
    .bind(candidate_json)
    .execute(&mut *connection)
    .await;
    let reenable = sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {schema}.wallet_nonce_candidates ENABLE TRIGGER USER"
    )))
    .execute(&mut *connection)
    .await;
    reenable.expect("re-enable candidate trigger after exact corruption restoration");
    assert_eq!(
        update
            .expect("restore exact candidate JSON after corruption probe")
            .rows_affected(),
        1
    );
}

async fn restore_completion_semantically(
    connection: &mut PgConnection,
    schema: &str,
    completion_key: &str,
    (request_json, completion_json): (String, String),
) {
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {schema}.wallet_nonce_completions DISABLE TRIGGER USER"
    )))
    .execute(&mut *connection)
    .await
    .expect("disable completion trigger for exact corruption restoration");
    let update = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {schema}.wallet_nonce_completions \
         SET request_json = $2, completion_json = $3 \
         WHERE semantic_completion_key = $1"
    )))
    .bind(completion_key)
    .bind(request_json)
    .bind(completion_json)
    .execute(&mut *connection)
    .await;
    let reenable = sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {schema}.wallet_nonce_completions ENABLE TRIGGER USER"
    )))
    .execute(&mut *connection)
    .await;
    reenable.expect("re-enable completion trigger after exact corruption restoration");
    assert_eq!(
        update
            .expect("restore exact completion JSON after corruption probe")
            .rows_affected(),
        1
    );
}

async fn assert_wallet_nonce_tables_empty(database_url: &str) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await
        .expect("open copied-target persistence probe");
    let rows = sqlx::query_scalar::<_, i64>(
        "SELECT (SELECT count(*) FROM wallet_nonce_domains) \
              + (SELECT count(*) FROM wallet_nonce_reservations) \
              + (SELECT count(*) FROM wallet_nonce_candidates) \
              + (SELECT count(*) FROM wallet_nonce_completions)",
    )
    .fetch_one(&pool)
    .await
    .expect("count copied-target wallet rows");
    assert_eq!(rows, 0, "rejected copied target must remain unmodified");
    pool.close().await;
}

async fn assert_wallet_nonce_schema_empty(database_url: &str, schema: &str) {
    let database_url =
        database_url_with_active_role(database_url, "mfm_evm_wallet_nonce_application");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("open exact-schema persistence probe");
    let schema = test_identifier(schema);
    let rows = sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!(
        "SELECT (SELECT count(*) FROM {schema}.wallet_nonce_domains) \
              + (SELECT count(*) FROM {schema}.wallet_nonce_reservations) \
              + (SELECT count(*) FROM {schema}.wallet_nonce_candidates) \
              + (SELECT count(*) FROM {schema}.wallet_nonce_completions)"
    )))
    .fetch_one(&pool)
    .await
    .expect("count exact-schema wallet rows");
    assert_eq!(rows, 0, "rejected deployment assembly must not persist");
    pool.close().await;
}

async fn assert_domain_absent(pool: &PgPool, domain_id: &str) {
    let present = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM wallet_nonce_domains WHERE wallet_nonce_domain_id = $1)",
    )
    .bind(domain_id)
    .fetch_one(pool)
    .await
    .expect("inspect absent wallet nonce domain");
    assert!(!present, "negative first use must not create domain state");
}

async fn assert_domain_high_water(pool: &PgPool, domain_id: &str, expected: u64) {
    let retained = sqlx::query_scalar::<_, String>(
        "SELECT local_high_water_nonce::text FROM wallet_nonce_domains \
         WHERE wallet_nonce_domain_id = $1",
    )
    .bind(domain_id)
    .fetch_one(pool)
    .await
    .expect("read retained wallet nonce high water");
    assert_eq!(retained, expected.to_string());
}

async fn assert_reservation_absent(pool: &PgPool, request: &ReserveEvmNonceRequest) {
    let present = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS ( \
             SELECT 1 FROM wallet_nonce_reservations \
             WHERE semantic_reservation_key = $1 \
         )",
    )
    .bind(request.reservation_key.as_str())
    .fetch_one(pool)
    .await
    .expect("inspect absent semantic reservation");
    assert!(
        !present,
        "negative disposition must not retain a reservation"
    );
}

async fn assert_candidate_absent(pool: &PgPool, request: &ActivateEvmCandidateRequest) {
    let present = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS ( \
             SELECT 1 FROM wallet_nonce_candidates \
             WHERE semantic_candidate_operation_key = $1 \
         )",
    )
    .bind(request.candidate_operation_key.as_str())
    .fetch_one(pool)
    .await
    .expect("inspect absent semantic candidate");
    assert!(
        !present,
        "candidate progression conflict must not retain a candidate"
    );
}

async fn assert_no_provider_wire_evidence(
    pool: &PgPool,
    provider_public_key: &str,
    provider_endpoint: &str,
) {
    const TABLES: [&str; 8] = [
        "wallet_store_schema_metadata",
        "wallet_store_incarnations",
        "wallet_store_lineage_heads",
        "wallet_domain_activations",
        "wallet_nonce_domains",
        "wallet_nonce_reservations",
        "wallet_nonce_candidates",
        "wallet_nonce_completions",
    ];
    for table in TABLES {
        let rows = sqlx::query_scalar::<_, String>(AssertSqlSafe(format!(
            "SELECT to_jsonb(retained)::text FROM {table} AS retained"
        )))
        .fetch_all(pool)
        .await
        .unwrap_or_else(|error| panic!("read redaction surface {table}: {error}"));
        for row in rows {
            assert!(!row.contains("\"provider_attestation\":"));
            assert!(!row.contains("\"target_attestation\":"));
            assert!(!row.contains("\"signature\":"));
            assert!(!row.contains(provider_public_key));
            assert!(!row.contains(provider_endpoint));
        }
    }
}

fn expected_table_privilege(role: &str, table: &str, privilege: &str) -> bool {
    if role == "mfm_evm_wallet_owner" {
        return true;
    }
    matches!(
        (role, table, privilege),
        (
            "mfm_evm_wallet_activation_admin",
            "wallet_store_schema_metadata"
                | "wallet_store_incarnations"
                | "wallet_store_lineage_heads"
                | "wallet_domain_activations",
            "SELECT",
        ) | (
            "mfm_evm_wallet_activation_admin",
            "wallet_store_incarnations"
                | "wallet_store_lineage_heads"
                | "wallet_domain_activations",
            "INSERT",
        ) | (
            "mfm_evm_wallet_activation_admin",
            "wallet_store_lineage_heads",
            "UPDATE"
        ) | (
            "mfm_evm_wallet_activation_public",
            "wallet_store_schema_metadata"
                | "wallet_store_incarnations"
                | "wallet_store_lineage_heads"
                | "wallet_domain_activations",
            "SELECT",
        ) | (
            "mfm_evm_wallet_nonce_application",
            "wallet_store_schema_metadata"
                | "wallet_nonce_domains"
                | "wallet_nonce_reservations"
                | "wallet_nonce_candidates"
                | "wallet_nonce_completions",
            "SELECT",
        ) | (
            "mfm_evm_wallet_nonce_application",
            "wallet_nonce_domains"
                | "wallet_nonce_reservations"
                | "wallet_nonce_candidates"
                | "wallet_nonce_completions",
            "INSERT",
        ) | (
            "mfm_evm_wallet_nonce_application",
            "wallet_nonce_domains",
            "UPDATE"
        ) | ("mfm_evm_wallet_test", _, "SELECT")
    )
}

async fn has_table_privilege(pool: &PgPool, role: &str, table: &str, privilege: &str) -> bool {
    sqlx::query_scalar("SELECT has_table_privilege($1, $2, $3)")
        .bind(role)
        .bind(table)
        .bind(privilege)
        .fetch_one(pool)
        .await
        .expect("inspect table privilege")
}

async fn has_schema_privilege(pool: &PgPool, role: &str, schema: &str, privilege: &str) -> bool {
    sqlx::query_scalar("SELECT has_schema_privilege($1, $2, $3)")
        .bind(role)
        .bind(schema)
        .bind(privilege)
        .fetch_one(pool)
        .await
        .expect("inspect schema privilege")
}

async fn has_function_privilege(
    pool: &PgPool,
    role: &str,
    function: &str,
    privilege: &str,
) -> bool {
    sqlx::query_scalar("SELECT has_function_privilege($1, $2, $3)")
        .bind(role)
        .bind(function)
        .bind(privilege)
        .fetch_one(pool)
        .await
        .expect("inspect function privilege")
}

struct Fixture {
    common_ref: EvmWalletReference,
    registry_lineage_ref: EvmWalletReference,
    provider_id: StableId,
    provider_fence_lineage_ref: EvmWalletReference,
    provider_fence_head_ref: EvmWalletReference,
    next_provider_fence_head_ref: EvmWalletReference,
    chain_attestation: ChainInstanceRegistryAttestation,
    routing_catalog: EvmRoutingCatalogDescriptor,
    refreshed_routing_catalog: EvmRoutingCatalogDescriptor,
    route_generation_ref: EvmWalletReference,
    redundant_route_generation_ref: mfm_evm::EvmRoutingGenerationRef,
    redundant_route_membership_ref: EvmWalletReference,
    nonce_domain: mfm_evm::WalletNonceDomain,
    incarnation: WalletNonceStoreIncarnation,
    next_incarnation: WalletNonceStoreIncarnation,
    activation_record: WalletNonceDomainActivationRecord,
    secondary_activation_record: WalletNonceDomainActivationRecord,
    secondary_nonce_domain: mfm_evm::WalletNonceDomain,
    current_public_head: HistoryObject,
    next_public_head: HistoryObject,
    next_lineage_head: WalletNonceStoreLineageHead,
    semantic_signer_id: StableId,
}

fn initial_incarnation(
    fixture: &Fixture,
    lineage_id: &str,
    target_id: &str,
) -> WalletNonceStoreIncarnation {
    WalletNonceStoreIncarnation {
        wallet_nonce_store_lineage_id: lineage_id.to_owned(),
        writer_epoch: 1,
        physical_target_instance_id: target_id.to_owned(),
        non_exportable_target_public_key_ref: fixture.common_ref.clone(),
        target_attestation_contract_ref: fixture.common_ref.clone(),
    }
}

fn activation_record_for_sender(
    fixture: &Fixture,
    sender_byte: u8,
    incarnation: &WalletNonceStoreIncarnation,
) -> WalletNonceDomainActivationRecord {
    let sender = Address::repeat_byte(sender_byte);
    let binding = fixture.chain_attestation.binding().expect("chain binding");
    let lineage =
        derive_evm_chain_lineage_id(binding.qualified_chain_instance_id()).expect("chain lineage");
    let nonce_domain = derive_wallet_nonce_domain(lineage, sender).expect("nonce domain");
    let mut record = fixture.activation_record.clone();
    record.wallet_nonce_store_lineage_id = incarnation.wallet_nonce_store_lineage_id.clone();
    record.initial_store_incarnation_ref =
        canonical_wallet_reference(incarnation).expect("initial incarnation reference");
    record.wallet_nonce_domain = nonce_domain.clone();
    record.sender_identity = format!("{sender:#x}");
    record.exhaustive_sender_path_inventory_digest =
        mfm_journal::structured::domain_content_digest(
            "mfm.evm.test/sender-path-inventory.v1",
            &nonce_domain,
        )
        .expect("sender inventory")
        .as_str()
        .to_owned();
    record.validate().expect("activation race record");
    record
}

impl Fixture {
    fn new() -> Self {
        let common_ref = evm_wallet_nonce_policy_ref().expect("wallet policy reference");
        let registry_lineage_ref = authority_reference("activation-registry-lineage");
        let provider_fence_lineage_ref = authority_reference("provider-fence-lineage");
        let chain_registry_lineage_ref = authority_reference("chain-registry-lineage");
        let chain_registry_head_ref = authority_reference("chain-registry-head-initial");
        let refreshed_chain_registry_head_ref =
            authority_reference("chain-registry-head-refreshed");
        let chain_registry_issuance_ref = authority_reference("chain-registry-issuance-primary");
        let route_membership_ref = authority_reference("route-membership-primary");
        let redundant_route_membership_ref = authority_reference("route-membership-redundant");
        let refreshed_route_membership_ref = authority_reference("route-membership-refreshed");
        let chain_declaration = ChainInstanceDeclaration::new(
            chain_registry_lineage_ref,
            stable("mfm.evm.test/chain-instance"),
            1,
            B256::repeat_byte(0x11),
            U256::from(100_u64),
            B256::repeat_byte(0x12),
        )
        .expect("chain declaration");
        let chain_attestation = ChainInstanceRegistryAttestation::new(
            chain_declaration,
            chain_registry_issuance_ref,
            chain_registry_head_ref.clone(),
        )
        .expect("chain attestation");
        let chain_binding = chain_attestation.binding().expect("chain binding");
        let chain_lineage =
            derive_evm_chain_lineage_id(chain_binding.qualified_chain_instance_id())
                .expect("chain lineage");
        let route_descriptor = EvmRoutingGenerationDescriptor::new(
            "wallet-test-network",
            "mfm.evm.json-rpc",
            stable("mfm.evm.test/route-generation"),
            route_membership_ref.clone(),
            chain_binding.clone(),
        )
        .expect("route descriptor");
        let route_generation = route_descriptor.generation_ref().expect("route generation");
        let route_generation_ref = EvmWalletReference::from_content_ref(
            route_generation
                .to_content_ref()
                .expect("route generation reference"),
        );
        let redundant_route_descriptor = EvmRoutingGenerationDescriptor::new(
            "wallet-test-network",
            "mfm.evm.json-rpc-redundant",
            stable("mfm.evm.test/route-generation-redundant"),
            redundant_route_membership_ref.clone(),
            chain_binding.clone(),
        )
        .expect("redundant route descriptor");
        let redundant_route_generation_ref = redundant_route_descriptor
            .generation_ref()
            .expect("redundant route generation");
        let routing_catalog = EvmRoutingCatalogDescriptor::new(
            chain_registry_head_ref,
            vec![chain_attestation.clone()],
            vec![route_descriptor.clone(), redundant_route_descriptor.clone()],
        )
        .expect("routing catalog");
        let refreshed_route_descriptor = EvmRoutingGenerationDescriptor::new(
            "wallet-test-network",
            "mfm.evm.json-rpc-refreshed",
            stable("mfm.evm.test/route-generation-refreshed"),
            refreshed_route_membership_ref,
            chain_binding,
        )
        .expect("refreshed route descriptor");
        let refreshed_routing_catalog = EvmRoutingCatalogDescriptor::new(
            refreshed_chain_registry_head_ref,
            vec![chain_attestation.clone()],
            vec![
                route_descriptor,
                redundant_route_descriptor,
                refreshed_route_descriptor,
            ],
        )
        .expect("refreshed routing catalog");
        let sender = Address::repeat_byte(0x21);
        let nonce_domain =
            derive_wallet_nonce_domain(chain_lineage.clone(), sender).expect("nonce domain");
        let secondary_sender = Address::repeat_byte(0x22);
        let secondary_nonce_domain =
            derive_wallet_nonce_domain(chain_lineage, secondary_sender).expect("secondary domain");
        let incarnation = WalletNonceStoreIncarnation {
            wallet_nonce_store_lineage_id: "mfm.evm.test/wallet-lineage".to_owned(),
            writer_epoch: 1,
            physical_target_instance_id: "mfm.evm.test/wallet-target-one".to_owned(),
            non_exportable_target_public_key_ref: common_ref.clone(),
            target_attestation_contract_ref: common_ref.clone(),
        };
        let next_incarnation = WalletNonceStoreIncarnation {
            wallet_nonce_store_lineage_id: incarnation.wallet_nonce_store_lineage_id.clone(),
            writer_epoch: 2,
            physical_target_instance_id: "mfm.evm.test/wallet-target-two".to_owned(),
            non_exportable_target_public_key_ref: common_ref.clone(),
            target_attestation_contract_ref: common_ref.clone(),
        };
        let initial_store_incarnation_ref =
            canonical_wallet_reference(&incarnation).expect("incarnation reference");
        let inventory = mfm_journal::structured::domain_content_digest(
            "mfm.evm.test/sender-path-inventory.v1",
            &nonce_domain,
        )
        .expect("inventory digest");
        let activation_record = WalletNonceDomainActivationRecord {
            activation_contract_ref: common_ref.clone(),
            qualified_activation_registry_lineage_ref: registry_lineage_ref.clone(),
            wallet_nonce_store_lineage_id: incarnation.wallet_nonce_store_lineage_id.clone(),
            initial_store_incarnation_ref,
            wallet_nonce_domain: nonce_domain.clone(),
            chain_instance_attestation: chain_attestation.clone(),
            initial_route_generation_ref: route_generation,
            initial_route_membership_issuance_ref: route_membership_ref,
            sender_identity: format!("{sender:#x}"),
            issuer_namespace_contract_ref: common_ref.clone(),
            replay_exclusion_contract_ref: common_ref.clone(),
            replay_exclusion_disposition:
                ReplayExclusionDisposition::EveryPriorRequestReplayAndRetryIngressExcluded,
            finalized_sender_nonce_floor: 7,
            finalized_block_number: "100".to_owned(),
            finalized_block_hash: format!("{:#x}", B256::repeat_byte(0x12)),
            qualified_observation_proof_ref: common_ref.clone(),
            exhaustive_sender_path_inventory_digest: inventory.as_str().to_owned(),
            exclusive_current_control: ExclusiveCurrentControl::EveryPriorWriterSignerRelayerOperatorStaleDeploymentAndDirectSubmitPathFenced,
            prior_effect_disposition: PriorEffectDisposition::NoUnresolvedPossibleEntry,
            prior_resource_disposition:
                PriorResourceDisposition::EveryPriorAllocationAndSubmittedCandidateTerminal,
            new_idempotency_epoch: "mfm.evm.test/idempotency-epoch".to_owned(),
        };
        let mut secondary_activation_record = activation_record.clone();
        secondary_activation_record.wallet_nonce_domain = secondary_nonce_domain.clone();
        secondary_activation_record.sender_identity = format!("{secondary_sender:#x}");
        secondary_activation_record.exhaustive_sender_path_inventory_digest =
            mfm_journal::structured::domain_content_digest(
                "mfm.evm.test/sender-path-inventory.v1",
                &secondary_nonce_domain,
            )
            .expect("secondary inventory digest")
            .as_str()
            .to_owned();
        secondary_activation_record
            .validate()
            .expect("secondary activation record");
        let current_public_head = history_object("current-head", 1);
        let next_public_head = history_object("next-head", 2);
        let provider_fence_head_ref =
            EvmWalletReference::from_content_ref(current_public_head.content_ref.clone());
        let next_provider_fence_head_ref =
            EvmWalletReference::from_content_ref(next_public_head.content_ref.clone());
        let next_lineage_head = WalletNonceStoreLineageHead {
            wallet_nonce_store_lineage_id: incarnation.wallet_nonce_store_lineage_id.clone(),
            writer_epoch: 2,
            public_lineage_head_ref: EvmWalletReference::from_content_ref(
                next_public_head.content_ref.clone(),
            ),
        };
        Self {
            common_ref,
            registry_lineage_ref,
            provider_id: stable("mfm.evm.test/wallet-authority-provider"),
            provider_fence_lineage_ref,
            provider_fence_head_ref,
            next_provider_fence_head_ref,
            chain_attestation,
            routing_catalog,
            refreshed_routing_catalog,
            route_generation_ref,
            redundant_route_generation_ref,
            redundant_route_membership_ref,
            nonce_domain,
            incarnation,
            next_incarnation,
            activation_record,
            secondary_activation_record,
            secondary_nonce_domain,
            current_public_head,
            next_public_head,
            next_lineage_head,
            semantic_signer_id: stable("mfm.evm.test/semantic-signer"),
        }
    }

    fn reserve_request(
        &self,
        domain_activation_attestation: WalletNonceDomainActivationAttestation,
        pending_nonce: u64,
        token: &str,
    ) -> ReserveEvmNonceRequest {
        self.reserve_request_for_principal(
            domain_activation_attestation,
            pending_nonce,
            token,
            stable("mfm.evm.test/principal"),
        )
    }

    fn reserve_request_for_principal(
        &self,
        domain_activation_attestation: WalletNonceDomainActivationAttestation,
        pending_nonce: u64,
        token: &str,
        principal: StableId,
    ) -> ReserveEvmNonceRequest {
        let transaction_intent = self.transaction_intent(token);
        let candidate_family = EvmCandidateFamily::new(
            &transaction_intent,
            vec![
                EvmWalletFeeCandidate::new(U256::from(20_u64), U256::from(2_u64))
                    .expect("initial fee candidate"),
                EvmWalletFeeCandidate::new(U256::from(30_u64), U256::from(3_u64))
                    .expect("replacement fee candidate"),
            ],
        )
        .expect("candidate family");
        let tenant = TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000051")
            .expect("tenant");
        let issuer = derive_authenticated_intent_issuer_id(
            &tenant,
            &principal,
            &self.activation_record.issuer_namespace_contract_ref,
        )
        .expect("issuer");
        let submission_intent_id =
            derive_submission_intent_id(&self.nonce_domain, &issuer, token).expect("intent id");
        let reservation_key =
            derive_evm_nonce_reservation_key(&self.nonce_domain, &submission_intent_id)
                .expect("reservation key");
        ReserveEvmNonceRequest {
            nonce_domain: self.nonce_domain.clone(),
            domain_activation_attestation,
            submission_intent_id,
            transaction_intent,
            candidate_family,
            reservation_key,
            qualified_floor: QualifiedPendingNonceFloor {
                observed: ObservedPendingNonceFloor {
                    nonce_domain: self.nonce_domain.clone(),
                    route_generation_ref: self.route_generation_ref.clone(),
                    pending_nonce,
                },
                pending_floor_policy_ref: self.common_ref.clone(),
            },
        }
    }

    fn transaction_intent(&self, token: &str) -> EvmTransactionIntent {
        let template = EvmWalletTransactionTemplate::new(
            EvmTransactionTarget::new(format!("wallet-test-{token}")).expect("transaction target"),
            EvmWalletTransactionAction::call(Address::repeat_byte(0x31)),
            U256::ZERO,
            [],
            Vec::new(),
            U256::from(21_000_u64),
        )
        .expect("transaction template");
        EvmTransactionIntent::new(
            self.chain_attestation.binding().expect("chain binding"),
            self.nonce_domain.clone(),
            template,
            self.semantic_signer_id.clone(),
            self.common_ref.clone(),
            self.common_ref.clone(),
            evm_wallet_assurance_policy_ref().expect("assurance policy"),
        )
        .expect("transaction intent")
    }

    fn activation_request(
        &self,
        reserve_request: &ReserveEvmNonceRequest,
        reservation: &mfm_evm::ReservedWalletNonce,
    ) -> (ActivateEvmCandidateRequest, String) {
        self.candidate_request(reserve_request, reservation, &[])
    }

    fn replacement_request(
        &self,
        reserve_request: &ReserveEvmNonceRequest,
        reservation: &mfm_evm::ReservedWalletNonce,
        activated_candidates: &[ActiveWalletCandidate],
    ) -> (ActivateEvmCandidateRequest, String) {
        assert!(!activated_candidates.is_empty());
        self.candidate_request(reserve_request, reservation, activated_candidates)
    }

    fn candidate_request(
        &self,
        reserve_request: &ReserveEvmNonceRequest,
        reservation: &mfm_evm::ReservedWalletNonce,
        activated_candidates: &[ActiveWalletCandidate],
    ) -> (ActivateEvmCandidateRequest, String) {
        let candidate_ordinal =
            u16::try_from(activated_candidates.len()).expect("candidate ordinal fits u16");
        let fee = reserve_request
            .candidate_family
            .candidates()
            .get(usize::from(candidate_ordinal))
            .expect("candidate fee")
            .clone();
        let envelope = reserve_request
            .transaction_intent
            .unsigned_candidate(reservation.nonce, &fee)
            .expect("unsigned envelope");
        let unsigned_candidate_digest = format!("{:#x}", envelope.signing_digest());
        let unsigned = UnsignedWalletCandidate {
            transaction_intent: reserve_request.transaction_intent.clone(),
            semantic_reservation_key: reservation.semantic_reservation_key.clone(),
            nonce: reservation.nonce,
            candidate_ordinal,
            fee,
            unsigned_candidate_digest: unsigned_candidate_digest.clone(),
        };
        let attested = AttestedWalletCandidate {
            semantic_reservation_key: reservation.semantic_reservation_key.clone(),
            candidate_ordinal,
            candidate_descriptor_ref: canonical_wallet_reference(&unsigned)
                .expect("candidate descriptor"),
            unsigned_candidate_digest: unsigned_candidate_digest.clone(),
            transaction_hash: format!(
                "{:#x}",
                B256::repeat_byte(
                    0x71_u8
                        .checked_add(
                            u8::try_from(candidate_ordinal).expect("candidate ordinal fits u8"),
                        )
                        .expect("candidate hash discriminator"),
                )
            ),
            semantic_signer_id: self.semantic_signer_id.as_str().to_owned(),
            signing_profile_contract_ref: reserve_request
                .transaction_intent
                .signing_profile_contract_ref()
                .clone(),
            signer_attestation_ref: self.common_ref.clone(),
        };
        (
            ActivateEvmCandidateRequest {
                nonce_domain: self.nonce_domain.clone(),
                candidate_operation_key: derive_evm_candidate_operation_key(
                    &reservation.semantic_reservation_key,
                    candidate_ordinal,
                )
                .expect("candidate key"),
                next_candidate: attested,
                activation_permit: derive_exact_candidate_activation_permit(
                    reservation,
                    activated_candidates,
                    candidate_ordinal,
                )
                .expect("candidate activation permit"),
            },
            unsigned_candidate_digest,
        )
    }

    fn completion_request(
        &self,
        reservation: &mfm_evm::ReservedWalletNonce,
        active_candidate: &mfm_evm::ActiveWalletCandidate,
    ) -> CompleteEvmNonceRequest {
        let transaction_hash = active_candidate.attested_candidate.transaction_hash.clone();
        let inclusion_block_hash = format!("{:#x}", B256::repeat_byte(0x72));
        let terminal_assurance_contract_ref =
            evm_wallet_assurance_policy_ref().expect("assurance policy");
        let canonical_public_result = "{\"execution_disposition\":\"succeeded\"}".to_owned();
        CompleteEvmNonceRequest {
            nonce_domain: self.nonce_domain.clone(),
            completion_key: derive_evm_nonce_completion_key(&reservation.semantic_reservation_key)
                .expect("completion key"),
            current_reservation: reservation.clone(),
            canonical_terminal_outcome: CanonicalTerminalOutcome {
                nonce_domain: self.nonce_domain.clone(),
                semantic_reservation_key: reservation.semantic_reservation_key.clone(),
                submission_intent_id: reservation.submission_intent_id.clone(),
                transaction_intent_digest: reservation.transaction_intent_digest.clone(),
                nonce: reservation.nonce,
                winning_candidate_ordinal: 0,
                winning_activation_evidence_ref: active_candidate.activation_evidence_ref.clone(),
                transaction_hash: transaction_hash.clone(),
                inclusion_block_number: "101".to_owned(),
                inclusion_block_hash: inclusion_block_hash.clone(),
                terminal_assurance_contract_ref: terminal_assurance_contract_ref.clone(),
                execution_disposition: ExecutionDisposition::Succeeded,
                canonical_public_result: canonical_public_result.clone(),
            },
            terminal_witnesses: TerminalWitnesses {
                transaction: EvmTransactionLookupObservation::Found {
                    transaction_hash: transaction_hash.clone(),
                    block_number: Some("101".to_owned()),
                    block_hash: Some(inclusion_block_hash.clone()),
                },
                receipt: EvmReceiptLookupObservation::Found {
                    transaction_hash,
                    block_number: "101".to_owned(),
                    block_hash: inclusion_block_hash.clone(),
                    status: 1,
                },
                finalized_head: EvmFinalizedHeadObservation {
                    block_number: "102".to_owned(),
                    block_hash: format!("{:#x}", B256::repeat_byte(0x73)),
                },
                inclusion_block: EvmInclusionBlockObservation {
                    block_number: "101".to_owned(),
                    block_hash: inclusion_block_hash,
                },
                terminal_assurance_contract_ref,
                canonical_public_result,
            },
        }
    }

    fn promotion_successor(&self) -> WalletNonceStoreSuccessor {
        WalletNonceStoreSuccessor {
            wallet_nonce_store_lineage_id: self.incarnation.wallet_nonce_store_lineage_id.clone(),
            expected_current_incarnation_ref: canonical_wallet_reference(&self.incarnation)
                .expect("current incarnation reference"),
            next_incarnation: self.next_incarnation.clone(),
        }
    }

    fn state_input(&self) -> LexicalValueRef {
        let reference = self
            .common_ref
            .to_content_ref()
            .expect("common content reference");
        LexicalValueRef {
            slot_ref: reference.clone(),
            value: TypedValueRef {
                contract_ref: reference.clone(),
                value_ref: reference,
            },
            structural_origin: None,
        }
    }

    fn refreshed_state_input(&self) -> LexicalValueRef {
        let reference = self
            .next_provider_fence_head_ref
            .to_content_ref()
            .expect("next head");
        LexicalValueRef {
            slot_ref: reference.clone(),
            value: TypedValueRef {
                contract_ref: reference.clone(),
                value_ref: reference,
            },
            structural_origin: None,
        }
    }
}

fn history_object(name: &str, generation: u64) -> HistoryObject {
    HistoryObject::new(
        stable(&format!("mfm.evm.test/{name}")),
        SchemaId::new(
            "mfm.evm.test-wallet-lineage-head",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.structured-schema.v1:mfm.evm.test-wallet-lineage-head:1"),
        )
        .expect("history schema"),
        format!("{{\"generation\":{generation}}}"),
    )
    .expect("history object")
}

fn authority_reference(role: &str) -> EvmWalletReference {
    let schema_id = SchemaId::new(
        "mfm.evm.test-wallet-authority-reference",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.evm.test-wallet-authority-reference.v1"),
    )
    .expect("authority reference schema");
    EvmWalletReference::from_content_ref(
        ContentRef::new(
            schema_id,
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(role.as_bytes()),
            ),
        )
        .expect("authority reference"),
    )
}

fn stable(value: &str) -> StableId {
    StableId::new(value).expect("stable id")
}

#[derive(Clone)]
struct WalletRoleDatabaseUrls {
    activation_admin: String,
    activation_public: String,
    nonce_application: String,
}

impl WalletRoleDatabaseUrls {
    fn for_schema(schema: &str) -> Self {
        Self {
            activation_admin: scoped_database_url(
                &required_database_url(ACTIVATION_ADMIN_DATABASE_URL_ENV),
                schema,
            ),
            activation_public: scoped_database_url(
                &required_database_url(ACTIVATION_PUBLIC_DATABASE_URL_ENV),
                schema,
            ),
            nonce_application: scoped_database_url(
                &required_database_url(NONCE_APPLICATION_DATABASE_URL_ENV),
                schema,
            ),
        }
    }

    fn for_database_and_schema(database: &str, schema: &str) -> Self {
        Self {
            activation_admin: scoped_database_url(
                &database_url_with_database(
                    &required_database_url(ACTIVATION_ADMIN_DATABASE_URL_ENV),
                    database,
                ),
                schema,
            ),
            activation_public: scoped_database_url(
                &database_url_with_database(
                    &required_database_url(ACTIVATION_PUBLIC_DATABASE_URL_ENV),
                    database,
                ),
                schema,
            ),
            nonce_application: scoped_database_url(
                &database_url_with_database(
                    &required_database_url(NONCE_APPLICATION_DATABASE_URL_ENV),
                    database,
                ),
                schema,
            ),
        }
    }

    fn through_proxy(&self, proxy_database_url: &str) -> Self {
        Self {
            activation_admin: database_url_with_login(proxy_database_url, &self.activation_admin),
            activation_public: database_url_with_login(proxy_database_url, &self.activation_public),
            nonce_application: database_url_with_login(proxy_database_url, &self.nonce_application),
        }
    }
}

fn required_database_url(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required for parity tests"))
}

fn database_url_with_login(database_url: &str, login_database_url: &str) -> String {
    let login = PgConnectOptions::from_str(login_database_url)
        .expect("parse role-specific PostgreSQL URL")
        .get_username()
        .to_owned();
    PgConnectOptions::from_str(database_url)
        .expect("parse proxied PostgreSQL URL")
        .username(&login)
        .to_url_lossy()
        .to_string()
}

fn database_url_with_active_role(database_url: &str, role: &str) -> String {
    let separator = if database_url.contains('?') { '&' } else { '?' };
    format!("{database_url}{separator}options%5Brole%5D={role}")
}

async fn configure_wallet_login_principals(
    admin: &mut PgConnection,
    role_urls: &WalletRoleDatabaseUrls,
) {
    LOGIN_PRINCIPALS_CONFIGURED
        .get_or_init(|| async {
            let principals = [
                (
                    database_login(&role_urls.activation_admin),
                    "mfm_evm_wallet_activation_admin",
                ),
                (
                    database_login(&role_urls.activation_public),
                    "mfm_evm_wallet_activation_public",
                ),
                (
                    database_login(&role_urls.nonce_application),
                    "mfm_evm_wallet_nonce_application",
                ),
            ];
            assert!(principals
                .iter()
                .all(|(principal, _)| principal != "postgres"));
            assert!(principals[0].0 != principals[1].0);
            assert!(principals[0].0 != principals[2].0);
            assert!(principals[1].0 != principals[2].0);

            for (principal, granted_role) in principals {
                let principal = test_identifier(&principal);
                let exists = sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = $1)",
                )
                .bind(principal.trim_matches('"'))
                .fetch_one(&mut *admin)
                .await
                .expect("inspect wallet login principal");
                if !exists {
                    sqlx::query(AssertSqlSafe(format!(
                        "CREATE ROLE {principal} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE \
                         NOINHERIT NOREPLICATION NOBYPASSRLS"
                    )))
                    .execute(&mut *admin)
                    .await
                    .expect("create wallet login principal");
                }
                sqlx::query(AssertSqlSafe(format!(
                    "ALTER ROLE {principal} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE \
                     NOINHERIT NOREPLICATION NOBYPASSRLS"
                )))
                .execute(&mut *admin)
                .await
                .expect("qualify wallet login principal attributes");

                let inherited_roles = sqlx::query_scalar::<_, String>(
                    "SELECT granted.rolname FROM pg_catalog.pg_auth_members AS membership \
                     JOIN pg_catalog.pg_roles AS granted ON granted.oid = membership.roleid \
                     JOIN pg_catalog.pg_roles AS member ON member.oid = membership.member \
                     WHERE member.rolname = $1 ORDER BY granted.rolname",
                )
                .bind(principal.trim_matches('"'))
                .fetch_all(&mut *admin)
                .await
                .expect("inventory wallet login memberships");
                for inherited_role in inherited_roles {
                    sqlx::query(AssertSqlSafe(format!(
                        "REVOKE {} FROM {principal}",
                        test_identifier(&inherited_role)
                    )))
                    .execute(&mut *admin)
                    .await
                    .expect("remove unrelated wallet login membership");
                }
                sqlx::query(AssertSqlSafe(format!(
                    "GRANT {} TO {principal} WITH ADMIN FALSE, INHERIT FALSE, SET TRUE",
                    test_identifier(granted_role)
                )))
                .execute(&mut *admin)
                .await
                .expect("grant exact wallet runtime role");
            }
        })
        .await;
}

fn database_login(database_url: &str) -> String {
    PgConnectOptions::from_str(database_url)
        .expect("parse role-specific database URL")
        .get_username()
        .to_owned()
}

fn scoped_database_url(base_url: &str, schema: &str) -> String {
    let separator = if base_url.contains('?') { '&' } else { '?' };
    format!("{base_url}{separator}options=-csearch_path%3D{schema}")
}

fn database_url_with_database(base_url: &str, database: &str) -> String {
    let (authority_and_path, query) = base_url
        .split_once('?')
        .map_or((base_url, None), |(head, tail)| (head, Some(tail)));
    let path_separator = authority_and_path
        .rfind('/')
        .expect("PostgreSQL URL contains a database path");
    let mut rewritten = format!("{}{database}", &authority_and_path[..=path_separator]);
    if let Some(query) = query {
        rewritten.push('?');
        rewritten.push_str(query);
    }
    rewritten
}

fn unique_schema() -> String {
    let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    format!(
        "mfm_evm_wallet_{}_{}_{}",
        std::process::id(),
        timestamp,
        counter
    )
}

fn unique_provider_socket_path() -> PathBuf {
    let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("mfm-wa-{}-{counter}.sock", std::process::id()))
}
