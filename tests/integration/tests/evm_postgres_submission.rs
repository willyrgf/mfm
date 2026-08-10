//! Managed-PostgreSQL qualification of the complete structured EVM path.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::Path;
use std::process::{Output, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alloy_primitives::{keccak256, Address, PrimitiveSignature, B256, U256};
use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use k256::ecdsa::SigningKey;
use mfm_app::{
    connect_production_application, AccessPolicyError, AccessTarget, AdmitRunRequest,
    AuthorizedTenant, EvmWalletDeployment, EvmWalletDeploymentAssemblyInput,
    EvmWalletDeploymentReleaseMaterial, ExportKind, ExportRequest, PageRequest, PublicJsonResponse,
    ReplayRequest, RunAccessGrant, RunAccessPolicy, SecretCredential,
};
use mfm_canonical::{limits::MAX_CANONICAL_JSON_BYTES, sha256_digest_bytes};
use mfm_certify::structured::ProgramRegistryBuilder;
use mfm_evm::{
    canonical_wallet_reference, derive_evm_chain_lineage_id, derive_evm_semantic_signer_id,
    derive_wallet_nonce_domain, evm_deterministic_signing_profile_ref,
    evm_submission_expansion_policy_ref, evm_wallet_assurance_policy_ref,
    evm_wallet_nonce_policy_ref, register_evm_submission_process,
    structured_evm_submission_entry_program, ActivateWalletCandidateCapability,
    AttestCandidateIdentityCapability, BroadcastExactCandidateCapability, BroadcastLineageHead,
    ChainInstanceDeclaration, ChainInstanceRegistryAttestation, CompleteWalletNonceCapability,
    CompletedWalletNonce, EvmBalanceAsset, EvmBalanceLaneInput, EvmBalanceSource,
    EvmBroadcastResource, EvmCallerSubmissionToken, EvmCandidateFamily, EvmCandidateSigner,
    EvmFinalizedHeadCapability, EvmInclusionBlockCapability, EvmNetworkBinding,
    EvmPendingNonceCapability, EvmReceiptLookupCapability, EvmRoutingCatalogDescriptor,
    EvmRoutingGenerationDescriptor, EvmSubmissionConfiguration, EvmSubmissionFailure,
    EvmSubmissionOutput, EvmSubmissionProcessQualification, EvmSubmissionRequest,
    EvmSubmitTransactionSelector, EvmTransactionIntent, EvmTransactionLookupCapability,
    EvmTransactionTarget, EvmWalletFeeCandidate, EvmWalletReference, EvmWalletTransactionAction,
    EvmWalletTransactionTemplate, ExclusiveCurrentControl, ExecutionDisposition,
    PriorEffectDisposition, PriorResourceDisposition, ReadWalletNonceStatusCapability,
    ReplayExclusionDisposition, ReserveWalletNonceCapability, WalletNonceAuthority,
    WalletNonceAuthorityResource, WalletNonceDomain, WalletNonceDomainActivationAttestation,
    WalletNonceDomainActivationRecord, WalletNonceStoreIncarnation,
    EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID, EVM_SUBMIT_TRANSACTION_OPERATION_ID,
    EVM_WALLET_REPLACEMENT_LIMIT,
};
use mfm_evm_live::transport::{EvmJsonRpcTransport, EvmRoutingCatalogBuilder, EvmRpcEndpoint};
use mfm_evm_live::{
    register_evm_live_submission_bindings, register_evm_wallet_authority_bindings,
    EvmPhysicalBindingReleaseHistory, EvmStructuredLiveBindings, EvmStructuredWalletBindings,
};
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, EntryPointId, InvocationIdentity,
    RunId, SchemaId, StableId, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    canonical_json, AccessKind, CommittedBatch, HistoryObject, LexicalValueRef, ObservationOutcome,
    PriorRunFactSourceManifest, RunRecord, ADMISSION_CONFIGURATION_OBJECT_TYPE,
    ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_keystore::{Keystore, KeystoreConfig, KeystoreSignerProvider};
use mfm_portfolio::{
    decode_portfolio_config, EvmRoutingBinding, ExecutionAnchor, PortfolioId,
    PortfolioPublicOutputs, PortfolioRoutingManifest, PortfolioSnapshotSelector,
    PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID, STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID,
};
use mfm_program::structured::{
    RuntimeEffectCapability, RuntimeReadCapability, RuntimeResourceAuthority, RuntimeSigner,
};
use mfm_replay::portable::PortableRunExport;
use mfm_runtime::history::StructuredAdmissionCommand;
use mfm_runtime::structured::DriveOutcome;
use mfm_signing::{
    GenerationGuardedDeterministicSigningProvider, PublicKeyBytes, PublicSigningIdentity,
    QualifiedReadSigningProvider, ReadAttestationQualificationFuture, SignatureBytes, SignerRef,
    SigningAlgorithmId, SigningError, SigningFuture, SigningGenerationGuard,
    SigningGenerationGuardFuture, SigningProfileId, SigningProviderError, SigningRequest,
    SigningResult, VerifiedGenerationGuardedSignerBinding,
    SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use mfm_spec::structured::structured_value_contract_ref;
use mfm_spec::structured::{
    ExpandedStructuredProgram, ExpansionStage, OperationOutcome, SecretFreeExecutableIdentity,
    SecretFreeImplementationManifest, SecretFreeQualificationArtifact, StructuredComponentKind,
    StructuredExpansionProfile,
};
use mfm_storage_evm_postgres::{
    open_activation_registry_admin, open_wallet_nonce_authority, PostgresEvmWalletSchema,
    QualifiedEvmRoutingCatalog, WalletAuthorityProviderClient, WalletAuthorityProviderTrust,
};
use mfm_storage_postgres::{
    migrate_test_database, open_configuration_maintenance, open_structured_authoritative,
    open_test_application_sessions, open_test_configuration_sessions, TestLoginCredential,
    TestTargetCredentials,
};
use mfm_store::structured::{
    ConfigurationAppendRequest, ConfigurationStreamKey, PhysicalBindingAuthorization,
    PhysicalBindingSupersession, ProposedCanonicalValue, PublicPhysicalBindingVerifier,
    RunEvidenceStatus, StructuredAdmissionMaterial, StructuredStoreError,
};
use ring::hmac;
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, ConnectOptions, PgConnection, PgPool, Row};
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;
use tower::ServiceExt as _;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use zeroize::Zeroizing;

use mfm_values::MfmValue;
use mfm_wallet_authority_provider_test_support::{
    postgres_proxy::{CommitFault, PostgresCommitFaultProxy},
    ProviderDeploymentAssemblyPolicy, ProviderProcess, ProviderProcessConfig,
    ProviderRpcInventoryTarget,
};

const WORKER_MODE_ENV: &str = "MFM_EVM_POSTGRES_SUBMISSION_MODE";
const HISTORY_SCHEMA_ENV: &str = "MFM_EVM_POSTGRES_HISTORY_SCHEMA";
const WALLET_SCHEMA_ENV: &str = "MFM_EVM_POSTGRES_WALLET_SCHEMA";
const RPC_ENDPOINT_ENV: &str = "MFM_EVM_POSTGRES_RPC_ENDPOINT";
const ACTIVATION_ENV: &str = "MFM_EVM_POSTGRES_ACTIVATION_ATTESTATION";
const PROVIDER_ENDPOINT_ENV: &str = "MFM_EVM_POSTGRES_PROVIDER_ENDPOINT";
const PROVIDER_PUBLIC_KEY_ENV: &str = "MFM_EVM_POSTGRES_PROVIDER_PUBLIC_KEY";
const WORKER_READY_FILE_ENV: &str = "MFM_EVM_POSTGRES_WORKER_READY_FILE";
const ACTIVATION_ADMIN_DATABASE_URL_ENV: &str = "MFM_EVM_WALLET_ACTIVATION_ADMIN_DATABASE_URL";
const ACTIVATION_PUBLIC_DATABASE_URL_ENV: &str = "MFM_EVM_WALLET_ACTIVATION_PUBLIC_DATABASE_URL";
const NONCE_APPLICATION_DATABASE_URL_ENV: &str = "MFM_EVM_WALLET_NONCE_APPLICATION_DATABASE_URL";
const ACTIVATION_ADMIN_ROLE: &str = "mfm_evm_wallet_activation_admin";
const PHASE_ADMIT_BROADCAST: &str = "admit-broadcast";
const PHASE_RESUME_COMPLETE: &str = "resume-complete";
const PHASE_PRODUCTION_ADMIT: &str = "production-admit";
const PHASE_PRODUCTION_RESUME: &str = "production-resume";
const PHASE_PRODUCTION_RECOVER: &str = "production-recover";
const INJECTED_CRASH_AFTER_BROADCAST_ENV: &str = "MFM_EVM_POSTGRES_INJECTED_CRASH_AFTER_BROADCAST";
const INJECTED_CRASH_AFTER_RECEIPT_ENV: &str = "MFM_EVM_POSTGRES_INJECTED_CRASH_AFTER_RECEIPT";
const INJECTED_CRASH_AFTER_FINALITY_ENV: &str = "MFM_EVM_POSTGRES_INJECTED_CRASH_AFTER_FINALITY";
const INJECTED_CRASH_AFTER_COMPLETION_ENV: &str =
    "MFM_EVM_POSTGRES_INJECTED_CRASH_AFTER_COMPLETION";
const SKIP_PRODUCTION_PROJECTION_VERIFY_ENV: &str =
    "MFM_EVM_POSTGRES_SKIP_PRODUCTION_PROJECTION_VERIFY";
const WORKER_STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
const COMPLETION_BOUNDARY_TIMEOUT: Duration = Duration::from_secs(1_200);
const PORTFOLIO_INVOCATION: &str = "00000000-0000-4000-8000-000000000061";
const SUBMISSION_INVOCATION: &str = "00000000-0000-4000-8000-000000000062";
const CROSS_CHAIN_INVOCATION: &str = "00000000-0000-4000-8000-000000000063";
const RECOVERY_SUBMISSION_INVOCATION: &str = "00000000-0000-4000-8000-000000000064";
const SUBMISSION_TOKEN: &str = "integration-submission";
const CROSS_CHAIN_SUBMISSION_TOKEN: &str = "cross-chain-submission";
const MAX_RETAINED_DOCUMENT_BYTES: usize = MAX_CANONICAL_JSON_BYTES;
// Keep the end-to-end fixture above the historical single-frame boundary even though the
// generated retained-frame ceiling now admits the larger 32 MiB certified-program envelope.
const HISTORICAL_SINGLE_FRAME_BYTES: usize = 16_777_216;
const BLOCK_NUMBER: &str = "0x64";
const BLOCK_HASH: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FINALIZED_NUMBER: &str = "0x65";
const FINALIZED_HASH: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const RECIPIENT: Address = Address::repeat_byte(0x31);
const MAX_DRIVES: usize = 4096;
const WORKER_STACK_BYTES: usize = 32 * 1024 * 1024;
const SECRET_CREDENTIAL_CANARY: &str =
    "mfm-secret-canary-credential-7f925fb6e9404da7b3cbf5be862221ba";
const PROVIDER_TEXT_CANARY: &str = "mfm-provider-canary-endpoint-81d386f8d1724bba95f1585538416486";
const RPC_TARGET_IDENTITY: [u8; 32] = [0x51; 32];
const RPC_PROOF_KEY: [u8; 32] = [0x72; 32];
const INTEGRATION_SIGNING_KEY: [u8; 32] = [0x07; 32];
const KEYSTORE_PASSWORD: &str = "integration-only-strong-password";

static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture_routing_policy(fixture: &Fixture) -> HistoryObject {
    HistoryObject::new(
        stable(ADMISSION_ROUTING_POLICY_OBJECT_TYPE),
        EvmRoutingCatalogDescriptor::schema_id().expect("routing catalog schema"),
        fixture
            .routing_catalog
            .canonical()
            .expect("canonical routing catalog")
            .as_str(),
    )
    .expect("routing policy object")
}

fn deployment_semantic_contract_refs() -> Vec<ContentRef> {
    vec![
        EvmCandidateSigner::contract()
            .and_then(|contract| contract.content_ref().map_err(Into::into))
            .expect("candidate signer contract"),
        evm_deterministic_signing_profile_ref()
            .and_then(|reference| reference.to_content_ref())
            .expect("signing profile contract"),
        BroadcastExactCandidateCapability::contract()
            .and_then(|contract| contract.content_ref().map_err(Into::into))
            .expect("broadcast contract"),
        structured_value_contract_ref::<EvmSubmissionConfiguration>()
            .expect("submission configuration contract"),
        evm_submission_expansion_policy_ref()
            .and_then(|reference| reference.to_content_ref())
            .expect("expansion contract"),
        evm_wallet_assurance_policy_ref()
            .and_then(|reference| reference.to_content_ref())
            .expect("assurance contract"),
    ]
}

fn provider_deployment_policy(fixture: &Fixture) -> ProviderDeploymentAssemblyPolicy {
    let policy = fixture_routing_policy(fixture);
    let rpc_certificate = certificate("rpc-certificate", 5);
    let signer_certificate = certificate("signer-certificate", 6);
    let signer_fence_certificate = certificate("signer-fence-certificate", 7);
    let broadcast_certificate = certificate("broadcast-certificate", 8);
    let broadcast_head_object = certificate("broadcast-head", 9);
    let wallet_read_certificate = certificate("wallet-read-certificate", 10);
    let wallet_effect_certificate = certificate("wallet-effect-certificate", 11);
    let balance_certificate = certificate("balance-certificate", 13);
    let route_target = fixture
        .route_generation_ref
        .to_content_ref()
        .expect("route target");
    let catalog_target = fixture
        .routing_catalog
        .content_ref()
        .expect("catalog target");
    let wallet_target = canonical_wallet_reference(&fixture.incarnation)
        .expect("wallet target reference")
        .to_content_ref()
        .expect("wallet target");
    let histories = [
        single_release_history(&policy.content_ref, route_target, rpc_certificate),
        single_release_history(
            &policy.content_ref,
            signer_certificate.content_ref.clone(),
            signer_certificate.clone(),
        ),
        single_release_history(
            &policy.content_ref,
            broadcast_head_object.content_ref.clone(),
            broadcast_certificate,
        ),
        single_release_history(&policy.content_ref, catalog_target, balance_certificate),
        single_release_history(
            &policy.content_ref,
            wallet_target.clone(),
            wallet_read_certificate,
        ),
        single_release_history(
            &policy.content_ref,
            wallet_target,
            wallet_effect_certificate,
        ),
    ];
    ProviderDeploymentAssemblyPolicy::new(
        fixture.semantic_signer_id.clone(),
        signer_certificate.content_ref,
        signer_fence_certificate.content_ref,
        certificate("rpc-certificate", 5).content_ref,
        deployment_semantic_contract_refs(),
        histories
            .iter()
            .map(|history| history.content_digest().expect("release history digest"))
            .collect(),
    )
    .expect("provider deployment policy")
}

#[tokio::test]
async fn qualified_evm_submission_production_restarts_after_one_broadcast_and_completes() {
    let database = TestDatabase::create().await;
    let fixture = Fixture::new();
    let wallet_url = database.activation_admin_url();
    let provider_directory = tempfile::tempdir().expect("create provider directory");
    let provider_config = ProviderProcessConfig::new(
        provider_directory.path().join("wallet-authority.sock"),
        database_url_with_active_role(&wallet_url, ACTIVATION_ADMIN_ROLE),
        database_url_with_active_role(
            &database.nonce_application_url(),
            "mfm_evm_wallet_nonce_application",
        ),
        database.wallet_schema.clone(),
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
        vec![fixture.activation_record.clone()],
        Vec::new(),
        None,
    )
    .expect("provider configuration")
    .with_rpc_inventory_targets(vec![ProviderRpcInventoryTarget::new(
        fixture
            .route_generation_ref
            .to_content_ref()
            .expect("provider route target"),
        RPC_TARGET_IDENTITY,
        RPC_PROOF_KEY,
    )])
    .expect("provider RPC inventory")
    .with_deployment_assembly_policies(vec![provider_deployment_policy(&fixture)])
    .expect("provider deployment policy");
    let mut provider = ProviderProcess::spawn(
        env!("CARGO_BIN_EXE_mfm-integration-wallet-authority-provider"),
        provider_config,
    )
    .expect("start separate wallet authority provider");
    let provider_trust = WalletAuthorityProviderTrust::new(
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
    .expect("provider trust anchor");
    let provider_client =
        WalletAuthorityProviderClient::connect_unix(provider.endpoint(), provider_trust)
            .await
            .expect("authenticate provider");
    provider_client
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify routing catalog");
    let registry =
        open_activation_registry_admin(&wallet_url, &database.wallet_schema, provider_client)
            .await
            .expect("open activation registry administrator");
    let activation = registry
        .issue_domain_activation(&fixture.activation_record, &fixture.incarnation)
        .await
        .expect("issue wallet-domain activation");
    drop(registry);

    let rpc = LoopbackRpc::start(fixture.sender).await;
    assert!(rpc.endpoint().contains(PROVIDER_TEXT_CANARY));
    // The first broadcast enters and is then answered ambiguously, which is the
    // production shape of a provider whose response is lost after entry. The
    // broadcast capability declares `EntryAbsorbing`, so the run must resolve
    // that in-run: commit the ambiguity, re-assert the byte-identical committed
    // request, and have the provider absorb the repeat. Exactly one transaction
    // must ever reach the provider across both invocations.
    rpc.answer_next_broadcasts_ambiguously(1);
    // The production application must own a fresh semantic intent from an
    // empty wallet authority. The deterministic worker is deliberately not
    // run here: doing so would pre-complete the same caller token and turn the
    // real keystore path into a read-only projection test.
    run_worker_expect_crash_after_broadcast(
        &database,
        rpc.endpoint(),
        PHASE_PRODUCTION_ADMIT,
        &activation,
        &provider,
    )
    .await;
    // Two invocations, one transaction: the repeat was absorbed.
    assert_eq!(rpc.operation_count("eth_sendRawTransaction"), 2);
    assert_eq!(rpc.accepted_transaction_count(), 1);

    rpc.enable_finality();
    run_worker_expect_crash_after_receipt(
        &database,
        rpc.endpoint(),
        PHASE_PRODUCTION_RESUME,
        &activation,
        &provider,
    )
    .await;
    // Two invocations, one transaction: the repeat was absorbed.
    assert_eq!(rpc.operation_count("eth_sendRawTransaction"), 2);
    assert_eq!(rpc.accepted_transaction_count(), 1);

    run_worker_expect_crash_after_finality(
        &database,
        rpc.endpoint(),
        PHASE_PRODUCTION_RESUME,
        &activation,
        &provider,
    )
    .await;
    // Two invocations, one transaction: the repeat was absorbed.
    assert_eq!(rpc.operation_count("eth_sendRawTransaction"), 2);
    assert_eq!(rpc.accepted_transaction_count(), 1);

    run_worker_expect_crash_before_completion_commit(
        &database,
        rpc.endpoint(),
        PHASE_PRODUCTION_RESUME,
        &activation,
        &mut provider,
    )
    .await;
    // Two invocations, one transaction: the repeat was absorbed.
    assert_eq!(rpc.operation_count("eth_sendRawTransaction"), 2);
    assert_eq!(rpc.accepted_transaction_count(), 1);

    run_worker_expect_completion_acknowledgement_loss(
        &database,
        rpc.endpoint(),
        PHASE_PRODUCTION_RECOVER,
        &activation,
        &provider,
    )
    .await;
    // Two invocations, one transaction: the repeat was absorbed.
    assert_eq!(rpc.operation_count("eth_sendRawTransaction"), 2);
    assert_eq!(rpc.accepted_transaction_count(), 1);

    run_worker_expect_crash_after_completion(
        &database,
        rpc.endpoint(),
        PHASE_PRODUCTION_RECOVER,
        &activation,
        &provider,
    )
    .await;
    // Two invocations, one transaction: the repeat was absorbed.
    assert_eq!(rpc.operation_count("eth_sendRawTransaction"), 2);
    assert_eq!(rpc.accepted_transaction_count(), 1);

    run_worker(
        &database,
        rpc.endpoint(),
        PHASE_PRODUCTION_RECOVER,
        &activation,
        &provider,
    )
    .await;
    // Two invocations, one transaction: the repeat was absorbed.
    assert_eq!(rpc.operation_count("eth_sendRawTransaction"), 2);
    assert_eq!(rpc.accepted_transaction_count(), 1);
    assert_eq!(rpc.operation_count("eth_getTransactionCount"), 1);
    assert!(rpc.operation_count("eth_chainId") >= 1);
    assert!(rpc.operation_count("eth_getBalance") >= 1);

    let submission_run_id = derive_application_run_id(
        &history_store_scope_id(&database.database_url, &database.history_schema).await,
        &fixture.tenant,
        &stable(EVM_SUBMIT_TRANSACTION_OPERATION_ID),
        &InvocationIdentity::new(RECOVERY_SUBMISSION_INVOCATION)
            .expect("recovery submission invocation identity"),
    );
    let original_submission_run_id = derive_application_run_id(
        &history_store_scope_id(&database.database_url, &database.history_schema).await,
        &fixture.tenant,
        &stable(EVM_SUBMIT_TRANSACTION_OPERATION_ID),
        &InvocationIdentity::new(SUBMISSION_INVOCATION)
            .expect("original submission invocation identity"),
    );
    let batches = database.history_batches(submission_run_id).await;
    let original_batches = database.history_batches(original_submission_run_id).await;
    assert!(
        batches.iter().any(|batch| {
            canonical_json(batch)
                .expect("canonical reconstructed batch")
                .as_bytes()
                .len()
                > HISTORICAL_SINGLE_FRAME_BYTES
        }),
        "qualification must exercise a full committed batch larger than the historical single-frame boundary"
    );
    let audit = HistoryAudit::from_batches(&batches);
    assert!(audit.closed);
    assert_eq!(audit.authorizations.len(), audit.observations.len());
    assert!(audit
        .observations
        .values()
        .all(|outcome| matches!(outcome, ObservationOutcome::Returned { .. })));
    let original_audit = HistoryAudit::from_batches(&original_batches);
    assert!(
        !original_audit.closed,
        "the pre-completion process-loss run must remain parked for recovery"
    );
    assert!(
        original_audit.authorizations.len() > original_audit.observations.len(),
        "the parked run must retain an unmatched completion authorization"
    );
    let mut audited_capabilities = original_audit.capability_refs.clone();
    audited_capabilities.extend(audit.capability_refs.iter().cloned());
    for capability in expected_access_capabilities() {
        assert!(
            audited_capabilities.contains(&capability),
            "missing audited capability {capability:?}"
        );
    }

    let outcome = audit
        .root_outcome::<EvmSubmissionOutput, EvmSubmissionFailure>(&batches)
        .expect("decode closed EVM root outcome");
    match outcome {
        OperationOutcome::Success(output) => assert_eq!(
            output.execution_disposition,
            ExecutionDisposition::Succeeded,
            "public output must expose only the canonical terminal disposition"
        ),
        OperationOutcome::Failure(failure) => panic!("submission failed: {failure:?}"),
    }
    let wallet_pool = database.wallet_pool().await;
    let completion_json =
        sqlx::query_scalar::<_, String>("SELECT completion_json FROM wallet_nonce_completions")
            .fetch_all(&wallet_pool)
            .await
            .expect("read persisted wallet completion");
    assert_eq!(completion_json.len(), 1);
    let persisted: CompletedWalletNonce =
        serde_json::from_str(&completion_json[0]).expect("decode persisted completion");
    assert_eq!(persisted.nonce, 7);
    assert_eq!(
        persisted.canonical_terminal_outcome.execution_disposition,
        ExecutionDisposition::Succeeded
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wallet_nonce_reservations")
            .fetch_one(&wallet_pool)
            .await
            .expect("count reservations"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wallet_nonce_candidates")
            .fetch_one(&wallet_pool)
            .await
            .expect("count candidates"),
        1
    );
    wallet_pool.close().await;
    database.assert_canaries_absent_from_persistence().await;

    rpc.shutdown().await;
    drop(provider);
    drop(provider_directory);
    database.cleanup().await;
}

#[tokio::test]
async fn production_keystore_fixture_matches_declared_signer_identity() {
    let fixture = Fixture::new();
    let signer_generation = certificate("signer-certificate", 6);
    let signer_fence = certificate("signer-fence-certificate", 7);
    let direct_submit_exclusion = certificate("rpc-certificate", 5);
    let (signer, _signer_directory) = qualified_production_signer(
        &fixture,
        &signer_generation,
        &signer_fence,
        &direct_submit_exclusion,
    )
    .await;
    let key =
        SigningKey::from_slice(&INTEGRATION_SIGNING_KEY).expect("fixed integration signing key");
    let expected_identity = PublicSigningIdentity::new(
        SigningAlgorithmId::new(SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID)
            .expect("public signing algorithm"),
        Some(
            PublicKeyBytes::new(
                key.verifying_key()
                    .to_encoded_point(true)
                    .as_bytes()
                    .to_vec(),
            )
            .expect("compressed integration public key"),
        ),
        Some(format!("{:#x}", fixture.sender)),
    )
    .expect("complete fixture signer identity");

    assert_eq!(signer.semantic_signer_id(), &fixture.semantic_signer_id);
    assert_eq!(
        signer.binding().expected_public_identity(),
        &expected_identity
    );
    assert_eq!(signing_address(&key), fixture.sender);
}

#[tokio::test]
async fn evm_postgres_submission_worker() {
    let Some(mode) = std::env::var_os(WORKER_MODE_ENV) else {
        return;
    };
    let mode = mode.to_str().expect("worker mode is UTF-8");
    let base_url = std::env::var("DATABASE_URL").expect("DATABASE_URL is required");
    let history_schema = std::env::var(HISTORY_SCHEMA_ENV).expect("history schema");
    let wallet_schema = std::env::var(WALLET_SCHEMA_ENV).expect("wallet schema");
    let rpc_endpoint = std::env::var(RPC_ENDPOINT_ENV).expect("RPC endpoint");
    let fixture = Fixture::new();
    let wallet_url = scoped_database_url(
        &required_database_url(NONCE_APPLICATION_DATABASE_URL_ENV),
        &wallet_schema,
    );
    let activation: mfm_evm::WalletNonceDomainActivationAttestation =
        serde_json::from_str(&std::env::var(ACTIVATION_ENV).expect("activation attestation"))
            .expect("decode activation attestation");
    let provider_trust = WalletAuthorityProviderTrust::new(
        fixture.provider_id.clone(),
        fixture.provider_fence_lineage_ref.clone(),
        fixture
            .chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        &std::env::var(PROVIDER_PUBLIC_KEY_ENV).expect("provider public key"),
    )
    .expect("provider trust anchor");
    let provider = WalletAuthorityProviderClient::connect_unix(
        std::env::var(PROVIDER_ENDPOINT_ENV).expect("provider endpoint"),
        provider_trust,
    )
    .await
    .expect("authenticate live provider");
    let qualified_routing_catalog = provider
        .qualify_routing_catalog(&fixture.routing_catalog)
        .await
        .expect("qualify live routing catalog");
    let wallet_authority = open_wallet_nonce_authority(
        &wallet_url,
        &wallet_schema,
        activation.clone(),
        fixture.incarnation.clone(),
        fixture.current_public_head.clone(),
        provider,
    )
    .await
    .expect("open real PostgreSQL wallet authority");
    if let Some(path) = std::env::var_os(WORKER_READY_FILE_ENV) {
        std::fs::write(path, b"wallet-authority-ready")
            .expect("publish wallet authority readiness");
    }
    if matches!(
        mode,
        PHASE_PRODUCTION_ADMIT | PHASE_PRODUCTION_RESUME | PHASE_PRODUCTION_RECOVER
    ) {
        run_production_application_worker(
            &base_url,
            &history_schema,
            &wallet_schema,
            &fixture,
            activation,
            &rpc_endpoint,
            wallet_authority,
            qualified_routing_catalog,
            mode,
        )
        .await;
        return;
    }
    let wallet_authority: Arc<dyn WalletNonceAuthority> = Arc::new(wallet_authority);
    let assembly = assemble_runtime(
        &fixture,
        activation,
        &rpc_endpoint,
        wallet_authority,
        mode == PHASE_ADMIT_BROADCAST,
    )
    .await;
    let sessions = open_test_application_sessions(TestTargetCredentials {
        schema_name: history_schema.clone(),
        run_reader: TestLoginCredential {
            database_url: std::env::var("MFM_TEST_RUN_READER_URL").expect("run reader url"),
        },
        run_writer: TestLoginCredential {
            database_url: std::env::var("MFM_TEST_RUN_WRITER_URL").expect("run writer url"),
        },
        configuration_reader: TestLoginCredential {
            database_url: std::env::var("MFM_TEST_CONFIG_READER_URL").expect("config reader url"),
        },
        configuration_writer: None,
    })
    .await
    .expect("issue history application sessions");
    let history_control = isolated_pool(&base_url, &history_schema).await;
    let assembled = open_structured_authoritative(
        sessions,
        assembly.registry,
        Arc::clone(&assembly.physical_verifier),
    )
    .await
    .expect("open authoritative structured history");
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let invocation = InvocationIdentity::new("00000000-0000-4000-8000-000000000051")
        .expect("invocation identity");
    let run_id = derive_application_run_id(
        &reader.store_identity().store_scope_id,
        &fixture.tenant,
        &operation_id(),
        &invocation,
    );

    match mode {
        PHASE_ADMIT_BROADCAST => {
            let phase_started = Instant::now();
            let (admitted_run_id, _attempt) = runtime
                .admit_run(StructuredAdmissionCommand::new(
                    fixture.tenant.clone(),
                    invocation,
                    operation_id(),
                    assembly.document,
                    assembly.admission_material,
                    vec![ProposedCanonicalValue::from_value(&assembly.request)
                        .expect("admission request value")],
                    AppendRequestId::new("evm-postgres-admit").expect("admission append id"),
                ))
                .await
                .expect("admit EVM submission");
            assert_eq!(admitted_run_id, run_id);
            let mut drive_count = 0_usize;
            for _ in 0..MAX_DRIVES {
                drive_count += 1;
                runtime
                    .drive_once(&run_id)
                    .await
                    .expect("drive before restart");
                if broadcast_observation_exists(
                    &history_control,
                    &run_id,
                    &broadcast_capability_ref(),
                )
                .await
                {
                    break;
                }
            }
            assert!(
                broadcast_observation_exists(
                    &history_control,
                    &run_id,
                    &broadcast_capability_ref(),
                )
                .await,
                "phase A did not persist the exact broadcast observation"
            );
            eprintln!(
                "structured EVM {mode}: {drive_count} drives in {} ms",
                phase_started.elapsed().as_millis()
            );
            assert!(!matches!(
                reader
                    .load_public(&run_id)
                    .await
                    .expect("verify pre-restart history")
                    .status(),
                RunEvidenceStatus::Closed
            ));
        }
        PHASE_RESUME_COMPLETE => {
            let phase_started = Instant::now();
            let mut closed = false;
            let mut drive_count = 0_usize;
            for _ in 0..MAX_DRIVES {
                drive_count += 1;
                match runtime
                    .drive_once(&run_id)
                    .await
                    .expect("drive after restart")
                {
                    DriveOutcome::Closed | DriveOutcome::TransitionCommitted { closed: true } => {
                        closed = true;
                        break;
                    }
                    DriveOutcome::PossibleEntry(_) | DriveOutcome::BlockedIntegrity => {
                        panic!("EVM submission parked before completion")
                    }
                    DriveOutcome::AccessObserved
                    | DriveOutcome::ConcurrentProgress
                    | DriveOutcome::TransitionCommitted { closed: false } => {}
                }
            }
            assert!(closed, "phase B did not close within its certified bound");
            eprintln!(
                "structured EVM {mode}: {drive_count} drives in {} ms",
                phase_started.elapsed().as_millis()
            );
            assert!(matches!(
                reader
                    .load_public(&run_id)
                    .await
                    .expect("verify closed history")
                    .status(),
                RunEvidenceStatus::Closed
            ));
        }
        _ => panic!("unknown worker mode"),
    }

    drop(reader);
    drop(runtime);
    history_control.close().await;
}

#[derive(Clone)]
struct AllowTenantPolicy {
    tenant: TenantScopeId,
}

#[async_trait::async_trait]
impl RunAccessPolicy for AllowTenantPolicy {
    async fn authorize(
        &self,
        credential: &SecretCredential,
        _grant: RunAccessGrant,
        _target: &AccessTarget,
    ) -> Result<AuthorizedTenant, AccessPolicyError> {
        if credential.expose_to_policy() != SECRET_CREDENTIAL_CANARY.as_bytes() {
            return Err(AccessPolicyError::AuthenticationRequired);
        }
        Ok(
            AuthorizedTenant::new(self.tenant.clone(), stable("mfm.evm.integration/principal"))
                .with_decision_ref(ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    sha256_digest_bytes(b"mfm.evm.integration/export-decision"),
                )),
        )
    }
}

struct ProductionDeploymentMaterial {
    wallet: EvmWalletDeployment,
    portfolio: mfm_portfolio::PortfolioConfig,
    submission: EvmSubmissionConfiguration,
    _signer_directory: tempfile::TempDir,
}

#[allow(clippy::too_many_arguments)]
async fn run_production_application_worker(
    base_url: &str,
    history_schema: &str,
    wallet_schema: &str,
    fixture: &Fixture,
    activation: mfm_evm::WalletNonceDomainActivationAttestation,
    rpc_endpoint: &str,
    wallet_authority: mfm_storage_evm_postgres::PostgresWalletNonceAuthority,
    qualified_routing_catalog: QualifiedEvmRoutingCatalog,
    mode: &str,
) {
    let material = production_deployment_material(
        fixture,
        activation,
        rpc_endpoint,
        wallet_authority,
        qualified_routing_catalog,
    )
    .await;
    let store_scope_id = history_store_scope_id(base_url, history_schema).await;
    if mode == PHASE_PRODUCTION_ADMIT {
        seed_production_configuration(
            base_url,
            history_schema,
            &store_scope_id,
            fixture,
            &material,
        )
        .await;
    }

    let application_sessions = open_test_application_sessions(TestTargetCredentials {
        schema_name: history_schema.to_owned(),
        run_reader: TestLoginCredential {
            database_url: std::env::var("MFM_TEST_RUN_READER_URL").expect("run reader url"),
        },
        run_writer: TestLoginCredential {
            database_url: std::env::var("MFM_TEST_RUN_WRITER_URL").expect("run writer url"),
        },
        configuration_reader: TestLoginCredential {
            database_url: std::env::var("MFM_TEST_CONFIG_READER_URL").expect("config reader url"),
        },
        configuration_writer: None,
    })
    .await
    .expect("issue production history sessions");
    let application = connect_production_application(
        application_sessions,
        Arc::new(AllowTenantPolicy {
            tenant: fixture.tenant.clone(),
        }),
        material.wallet,
    )
    .await
    .expect("connect the real production application");
    application
        .check_ready()
        .await
        .expect("production application readiness");
    assert_eq!(application.entry_points().len(), 2);
    assert!(application
        .entry_points()
        .iter()
        .any(|entry| { entry.entry_point_id().as_str() == PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID }));
    assert!(application
        .entry_points()
        .iter()
        .any(|entry| { entry.entry_point_id().as_str() == EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID }));

    let portfolio_invocation =
        InvocationIdentity::new(PORTFOLIO_INVOCATION).expect("portfolio invocation identity");
    let submission_invocation =
        InvocationIdentity::new(SUBMISSION_INVOCATION).expect("submission invocation identity");
    let portfolio_run_id = derive_application_run_id(
        &store_scope_id,
        &fixture.tenant,
        &stable(STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID),
        &portfolio_invocation,
    );
    let submission_run_id = derive_application_run_id(
        &store_scope_id,
        &fixture.tenant,
        &stable(EVM_SUBMIT_TRANSACTION_OPERATION_ID),
        &submission_invocation,
    );
    let history_control = isolated_pool(base_url, history_schema).await;

    match mode {
        PHASE_PRODUCTION_ADMIT => {
            let cross_chain_submission = fixture.cross_chain_submission_request();
            let before_wallet_rows = wallet_mutation_counts(base_url, wallet_schema).await;
            let cross_chain_error = application
                .admit_run(
                    credential(),
                    admission_request(
                        EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID,
                        InvocationIdentity::new(CROSS_CHAIN_INVOCATION)
                            .expect("cross-chain invocation identity"),
                        &EvmSubmitTransactionSelector::new(
                            cross_chain_submission
                                .transaction_intent()
                                .template()
                                .target()
                                .clone(),
                            EvmCallerSubmissionToken::new(CROSS_CHAIN_SUBMISSION_TOKEN)
                                .expect("cross-chain caller token"),
                        ),
                    ),
                )
                .await
                .expect_err("chain-B activation with route A must fail before admission");
            assert_eq!(cross_chain_error.code(), "ConfiguredValueInvalid");
            assert_canaries_absent(
                "cross-chain public error",
                canonical_json(&cross_chain_error)
                    .expect("canonical cross-chain public error")
                    .as_bytes(),
            );
            assert_eq!(
                wallet_mutation_counts(base_url, wallet_schema).await,
                before_wallet_rows,
                "cross-chain admission rejection must not mutate wallet authority state"
            );

            let portfolio_selector =
                PortfolioSnapshotSelector::new(material.portfolio.portfolio_id.clone());
            let portfolio_admission = application
                .admit_run(
                    credential(),
                    admission_request(
                        PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
                        portfolio_invocation,
                        &portfolio_selector,
                    ),
                )
                .await
                .expect("admit production portfolio snapshot");
            assert_eq!(
                response_run_id(&portfolio_admission),
                portfolio_run_id,
                "portfolio response must bind the derived run identity"
            );

            let submission_selector = EvmSubmitTransactionSelector::new(
                material
                    .submission
                    .transaction_intent()
                    .template()
                    .target()
                    .clone(),
                EvmCallerSubmissionToken::new(SUBMISSION_TOKEN).expect("submission caller token"),
            );
            let submission_admission = application
                .admit_run(
                    credential(),
                    admission_request(
                        EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID,
                        submission_invocation,
                        &submission_selector,
                    ),
                )
                .await
                .expect("admit production EVM submission");
            assert_eq!(
                response_run_id(&submission_admission),
                submission_run_id,
                "submission response must bind the derived run identity"
            );

            application
                .drive_once(credential(), portfolio_run_id.clone())
                .await
                .expect("advance production portfolio before restart");
            let broadcast_capability = broadcast_capability_ref();
            let mut observed_broadcast = false;
            for _ in 0..MAX_DRIVES {
                application
                    .drive_once(credential(), submission_run_id.clone())
                    .await
                    .expect("advance production submission before restart");
                if broadcast_observation_exists(
                    &history_control,
                    &submission_run_id,
                    &broadcast_capability,
                )
                .await
                {
                    observed_broadcast = true;
                    break;
                }
            }
            assert!(
                observed_broadcast,
                "production application did not persist the exact broadcast observation"
            );
            if std::env::var_os(INJECTED_CRASH_AFTER_BROADCAST_ENV).is_some() {
                std::process::exit(137);
            }
            let portfolio_view = application
                .read_public_run(credential(), portfolio_run_id)
                .await
                .expect("read open portfolio run before restart")
                .public_json()
                .expect("render open portfolio run");
            assert_ne!(portfolio_view["status"], "closed");
        }
        PHASE_PRODUCTION_RESUME => {
            drive_application_to_closed(&application, &portfolio_run_id).await;
            let crash_after_receipt = std::env::var_os(INJECTED_CRASH_AFTER_RECEIPT_ENV).is_some();
            let crash_after_finality =
                std::env::var_os(INJECTED_CRASH_AFTER_FINALITY_ENV).is_some();
            let crash_after_completion =
                std::env::var_os(INJECTED_CRASH_AFTER_COMPLETION_ENV).is_some();
            if crash_after_receipt || crash_after_finality || crash_after_completion {
                drive_application_to_closed_with_injected_crash(
                    &application,
                    &submission_run_id,
                    &history_control,
                    crash_after_receipt,
                    crash_after_finality,
                    crash_after_completion,
                )
                .await;
            } else {
                drive_application_to_closed(&application, &submission_run_id).await;
            }
            let wallet_pool = isolated_pool(base_url, wallet_schema).await;
            let completion_json = sqlx::query_scalar::<_, String>(
                "SELECT completion_json FROM wallet_nonce_completions",
            )
            .fetch_one(&wallet_pool)
            .await
            .expect("load exact production wallet completion");
            wallet_pool.close().await;
            let expected_completion: CompletedWalletNonce = serde_json::from_str(&completion_json)
                .expect("decode exact production wallet completion");
            verify_production_projections(
                application,
                &portfolio_run_id,
                &submission_run_id,
                &material.portfolio,
                &expected_completion,
            )
            .await;
        }
        PHASE_PRODUCTION_RECOVER => {
            // The portfolio run was closed before submission recovery began. The
            // original submission run may remain parked at the possible-entry
            // boundary after process loss, so recovery must admit a fresh run
            // against the retained wallet intent instead of re-driving that run.
            let recovery_invocation = InvocationIdentity::new(RECOVERY_SUBMISSION_INVOCATION)
                .expect("recovery submission invocation identity");
            let recovery_selector = EvmSubmitTransactionSelector::new(
                material
                    .submission
                    .transaction_intent()
                    .template()
                    .target()
                    .clone(),
                EvmCallerSubmissionToken::new(SUBMISSION_TOKEN).expect("recovery caller token"),
            );
            let recovery_admission = application
                .admit_run(
                    credential(),
                    admission_request(
                        EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID,
                        recovery_invocation.clone(),
                        &recovery_selector,
                    ),
                )
                .await
                .expect("admit recovery EVM submission");
            let recovery_run_id = response_run_id(&recovery_admission);
            assert_eq!(
                recovery_run_id,
                derive_application_run_id(
                    &store_scope_id,
                    &fixture.tenant,
                    &stable(EVM_SUBMIT_TRANSACTION_OPERATION_ID),
                    &recovery_invocation,
                ),
                "recovery response must bind the derived run identity"
            );
            let crash_after_receipt = std::env::var_os(INJECTED_CRASH_AFTER_RECEIPT_ENV).is_some();
            let crash_after_finality =
                std::env::var_os(INJECTED_CRASH_AFTER_FINALITY_ENV).is_some();
            let crash_after_completion =
                std::env::var_os(INJECTED_CRASH_AFTER_COMPLETION_ENV).is_some();
            if crash_after_receipt || crash_after_finality || crash_after_completion {
                drive_application_to_closed_with_injected_crash(
                    &application,
                    &recovery_run_id,
                    &history_control,
                    crash_after_receipt,
                    crash_after_finality,
                    crash_after_completion,
                )
                .await;
            } else {
                drive_application_to_closed(&application, &recovery_run_id).await;
            }
            let wallet_pool = isolated_pool(base_url, wallet_schema).await;
            let completion_json = sqlx::query_scalar::<_, String>(
                "SELECT completion_json FROM wallet_nonce_completions",
            )
            .fetch_one(&wallet_pool)
            .await
            .expect("load exact recovery wallet completion");
            wallet_pool.close().await;
            let expected_completion: CompletedWalletNonce = serde_json::from_str(&completion_json)
                .expect("decode exact recovery wallet completion");
            if std::env::var_os(SKIP_PRODUCTION_PROJECTION_VERIFY_ENV).is_none() {
                verify_production_projections(
                    application,
                    &portfolio_run_id,
                    &recovery_run_id,
                    &material.portfolio,
                    &expected_completion,
                )
                .await;
            }
        }
        _ => panic!("unknown production application worker mode"),
    }
    history_control.close().await;
}

async fn production_deployment_material(
    fixture: &Fixture,
    activation: mfm_evm::WalletNonceDomainActivationAttestation,
    rpc_endpoint: &str,
    wallet_authority: mfm_storage_evm_postgres::PostgresWalletNonceAuthority,
    qualified_routing_catalog: QualifiedEvmRoutingCatalog,
) -> ProductionDeploymentMaterial {
    let routing_policy = qualified_routing_catalog
        .routing_policy_object()
        .expect("qualified routing policy");
    let context_manifest = admission_object(
        ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
        "mfm.evm.integration.context",
        2,
    );
    let prior_run_source_manifest = PriorRunFactSourceManifest::new(Vec::new())
        .and_then(|manifest| manifest.to_history_object())
        .expect("prior-run source manifest");
    let rpc_certificate = certificate("rpc-certificate", 5);
    let signer_certificate = certificate("signer-certificate", 6);
    let signer_fence_certificate = certificate("signer-fence-certificate", 7);
    let broadcast_certificate = certificate("broadcast-certificate", 8);
    let broadcast_head_object = certificate("broadcast-head", 9);
    let wallet_read_certificate = certificate("wallet-read-certificate", 10);
    let wallet_effect_certificate = certificate("wallet-effect-certificate", 11);
    let balance_certificate = certificate("balance-certificate", 13);

    let chain_binding = fixture.chain_attestation.binding().expect("chain binding");
    let mut routes = EvmRoutingCatalogBuilder::new(
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        vec![fixture.chain_attestation.clone()],
    )
    .expect("routing catalog builder");
    let route = routes
        .insert(
            fixture.routing_descriptor.clone(),
            EvmRpcEndpoint::new(rpc_endpoint).expect("loopback endpoint"),
            None,
        )
        .expect("insert production exact route");
    assert_eq!(
        route,
        fixture
            .routing_descriptor
            .generation_ref()
            .expect("qualified route reference")
    );
    let route_wallet_ref = fixture.route_generation_ref.clone();
    let pending_rpc_inventory = mfm_evm_live::transport::PendingEvmRpcInventory::new(
        routes.build().expect("routing catalog"),
    )
    .expect("pending EVM RPC inventory");
    let submission = fixture.submission_configuration(activation, route_wallet_ref.clone());
    let (signer, signer_directory) = qualified_production_signer(
        fixture,
        &signer_certificate,
        &signer_fence_certificate,
        &rpc_certificate,
    )
    .await;
    let rpc_target_ref = route_wallet_ref
        .to_content_ref()
        .expect("RPC target reference");
    let signer_target_ref = signer.binding().durable_generation_ref().clone();
    let broadcast_target_ref = broadcast_head_object.content_ref.clone();
    let balance_target_ref = pending_rpc_inventory
        .routing_catalog_descriptor()
        .content_ref()
        .expect("balance catalog target reference");
    let wallet_target_ref = wallet_authority
        .domain_activation_attestation()
        .initial_store_incarnation_ref
        .to_content_ref()
        .expect("wallet target reference");
    let routing_manifest = PortfolioRoutingManifest::new(vec![EvmRoutingBinding::new(
        "integration-network",
        chain_binding.clone(),
        route.clone(),
    )
    .expect("portfolio routing binding")])
    .expect("portfolio routing manifest");
    let balance_binding = EvmNetworkBinding::new("integration-network", chain_binding, route)
        .expect("balance binding");
    let portfolio_coverage_inputs = vec![EvmBalanceLaneInput::new(
        0,
        "{}".to_owned(),
        balance_binding,
        18,
        EvmBalanceSource::new(fixture.sender, EvmBalanceAsset::Native)
            .expect("production native source"),
    )
    .expect("production balance coverage lane")];
    let portfolio = decode_portfolio_config(&json!({
        "portfolio_id": "production_portfolio",
        "quote_codes": ["USD"],
        "networks": [{
            "network_id": "integration-network",
            "family": "evm",
            "chain_id": 1,
            "native_decimals": 18,
            "metadata": {}
        }],
        "wallets": [{
            "wallet_id": "production_wallet",
            "subject": {"kind": "evm_address", "address": format!("{:#x}", fixture.sender)},
            "network_id": "integration-network",
            "implementation": {"kind": "address_only"},
            "symbol_ids": ["native.integration-network"],
            "metadata": {}
        }],
        "symbol_configs": [{
            "symbol_id": "native.integration-network",
            "display_symbol": "NATIVE",
            "network_id": "integration-network",
            "source": {"kind": "native"},
            "valuation": {"quotes": [{
                "quote": "USD",
                "priced_symbol_id": "native.integration-network",
                "unit_price_dec": "1.00"
            }]},
            "metadata": {}
        }],
        "metadata": {}
    }))
    .expect("production portfolio configuration");
    let releases = EvmWalletDeploymentReleaseMaterial::new(
        single_release_history(&routing_policy.content_ref, rpc_target_ref, rpc_certificate),
        single_release_history(
            &routing_policy.content_ref,
            signer_target_ref,
            signer_certificate,
        ),
        single_release_history(
            &routing_policy.content_ref,
            broadcast_target_ref,
            broadcast_certificate,
        ),
        single_release_history(
            &routing_policy.content_ref,
            balance_target_ref,
            balance_certificate,
        ),
        single_release_history(
            &routing_policy.content_ref,
            wallet_target_ref.clone(),
            wallet_read_certificate,
        ),
        single_release_history(
            &routing_policy.content_ref,
            wallet_target_ref,
            wallet_effect_certificate,
        ),
        BroadcastLineageHead {
            lineage_id: "mfm.evm.integration/broadcast-lineage".to_owned(),
            generation: 1,
            public_lineage_head_ref: EvmWalletReference::from_content_ref(
                broadcast_head_object.content_ref.clone(),
            ),
        },
        broadcast_head_object,
    )
    .expect("deployment release material");
    let assembly = EvmWalletDeploymentAssemblyInput::new(
        routing_manifest,
        qualified_routing_catalog,
        pending_rpc_inventory,
        wallet_authority,
        signer,
        submission.clone(),
        context_manifest,
        prior_run_source_manifest,
        portfolio_coverage_inputs,
        releases,
    )
    .expect("deployment assembly input");
    let wallet = EvmWalletDeployment::assemble(assembly)
        .await
        .expect("qualified production wallet deployment");
    ProductionDeploymentMaterial {
        wallet,
        portfolio,
        submission,
        _signer_directory: signer_directory,
    }
}

async fn history_store_scope_id(base_url: &str, history_schema: &str) -> StoreScopeId {
    let pool = isolated_pool(base_url, history_schema).await;
    let value = sqlx::query_scalar::<_, String>(
        "SELECT store_scope_id FROM store_identity WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .expect("load structured store scope");
    pool.close().await;
    StoreScopeId::new(value).expect("qualified structured store scope")
}

async fn seed_production_configuration(
    _base_url: &str,
    history_schema: &str,
    store_scope_id: &StoreScopeId,
    fixture: &Fixture,
    material: &ProductionDeploymentMaterial,
) {
    let writer = open_configuration_maintenance(
        open_test_configuration_sessions(
            history_schema.to_owned(),
            TestLoginCredential {
                database_url: std::env::var("MFM_TEST_CONFIG_READER_URL").expect("config reader"),
            },
            TestLoginCredential {
                database_url: std::env::var("MFM_TEST_CONFIG_WRITER_URL").expect("config writer"),
            },
        )
        .await
        .expect("issue configuration maintenance sessions"),
    )
    .await
    .expect("open deployment configuration maintenance");
    writer
        .append(ConfigurationAppendRequest::new(
            ConfigurationStreamKey::new(
                store_scope_id.clone(),
                fixture.tenant.clone(),
                stable(STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID),
                stable(material.portfolio.portfolio_id.as_str()),
            ),
            None,
            AppendRequestId::new("production-portfolio-configuration")
                .expect("portfolio configuration append id"),
            mfm_spec::structured::structured_value_contract_ref::<mfm_portfolio::PortfolioConfig>()
                .expect("portfolio value contract"),
            ProposedCanonicalValue::from_value(&material.portfolio)
                .expect("portfolio configured value"),
        ))
        .await
        .expect("append production portfolio configuration");
    writer
        .append(ConfigurationAppendRequest::new(
            ConfigurationStreamKey::new(
                store_scope_id.clone(),
                fixture.tenant.clone(),
                stable(EVM_SUBMIT_TRANSACTION_OPERATION_ID),
                stable(
                    material
                        .submission
                        .transaction_intent()
                        .template()
                        .target()
                        .as_str(),
                ),
            ),
            None,
            AppendRequestId::new("production-submission-configuration")
                .expect("submission configuration append id"),
            mfm_spec::structured::structured_value_contract_ref::<EvmSubmissionConfiguration>()
                .expect("submission value contract"),
            ProposedCanonicalValue::from_value(&material.submission)
                .expect("submission configured value"),
        ))
        .await
        .expect("append production submission configuration");
    let cross_chain_submission = fixture.cross_chain_submission_configuration();
    writer
        .append(ConfigurationAppendRequest::new(
            ConfigurationStreamKey::new(
                store_scope_id.clone(),
                fixture.tenant.clone(),
                stable(EVM_SUBMIT_TRANSACTION_OPERATION_ID),
                stable(
                    cross_chain_submission
                        .transaction_intent()
                        .template()
                        .target()
                        .as_str(),
                ),
            ),
            None,
            AppendRequestId::new("production-cross-chain-submission-configuration")
                .expect("cross-chain submission configuration append id"),
            mfm_spec::structured::structured_value_contract_ref::<EvmSubmissionConfiguration>()
                .expect("submission value contract"),
            ProposedCanonicalValue::from_value(&cross_chain_submission)
                .expect("cross-chain submission configured value"),
        ))
        .await
        .expect("append cross-chain submission configuration");
}

async fn wallet_mutation_counts(base_url: &str, wallet_schema: &str) -> (i64, i64, i64, i64) {
    let pool = isolated_pool(base_url, wallet_schema).await;
    let row = sqlx::query(
        "SELECT (SELECT count(*) FROM wallet_nonce_domains) AS domains, \
                (SELECT count(*) FROM wallet_nonce_reservations) AS reservations, \
                (SELECT count(*) FROM wallet_nonce_candidates) AS candidates, \
                (SELECT count(*) FROM wallet_nonce_completions) AS completions",
    )
    .fetch_one(&pool)
    .await
    .expect("count wallet authority mutation rows");
    let counts = (
        row.get("domains"),
        row.get("reservations"),
        row.get("candidates"),
        row.get("completions"),
    );
    pool.close().await;
    counts
}

fn admission_request<T>(
    entry_point_id: &str,
    invocation_identity: InvocationIdentity,
    input: &T,
) -> AdmitRunRequest
where
    T: serde::Serialize,
{
    AdmitRunRequest::new(
        EntryPointId::new(entry_point_id).expect("entry-point identity"),
        invocation_identity,
        mfm_spec::CanonicalJsonValue::new(
            serde_json::to_value(input).expect("serialize admission selector"),
        )
        .expect("canonical admission selector"),
    )
    .expect("construct admission request")
}

fn credential() -> SecretCredential {
    SecretCredential::new(SECRET_CREDENTIAL_CANARY.as_bytes().to_vec())
        .expect("canary test credential")
}

fn response_run_id(response: &mfm_app::AdmitRunResponse) -> RunId {
    let rendered = response.public_json().expect("render admission response");
    serde_json::from_value(rendered["run_id"].clone()).expect("decode admission run id")
}

fn derive_application_run_id(
    store_scope_id: &StoreScopeId,
    tenant_scope_id: &TenantScopeId,
    operation_id: &StableId,
    invocation_identity: &InvocationIdentity,
) -> RunId {
    mfm_journal::structured::derive_run_id(
        store_scope_id,
        tenant_scope_id,
        operation_id,
        invocation_identity,
    )
    .expect("derive production run identity")
}

async fn drive_application_to_closed(application: &mfm_app::Application, run_id: &RunId) {
    for _ in 0..MAX_DRIVES {
        let response = application
            .drive_once(credential(), run_id.clone())
            .await
            .expect("drive production application run");
        let rendered = response.public_json().expect("render drive response");
        match rendered["kind"].as_str() {
            Some("closed") => return,
            Some("advanced") => {}
            Some("waiting") => {
                panic!("production application run {run_id:?} parked before completion: {rendered}")
            }
            other => panic!("unexpected production drive outcome: {other:?}"),
        }
    }
    panic!("production application run did not close within its certified bound");
}

async fn drive_application_to_closed_with_injected_crash(
    application: &mfm_app::Application,
    run_id: &RunId,
    history_control: &PgPool,
    crash_after_receipt: bool,
    crash_after_finality: bool,
    crash_after_completion: bool,
) {
    let receipt_capability = read_capability_ref::<EvmReceiptLookupCapability>();
    let finality_capability = read_capability_ref::<EvmFinalizedHeadCapability>();
    for _ in 0..MAX_DRIVES {
        let response = application
            .drive_once(credential(), run_id.clone())
            .await
            .expect("drive production application run");
        if crash_after_receipt
            && capability_observation_exists(history_control, run_id, &receipt_capability).await
        {
            history_control.close().await;
            std::process::exit(137);
        }
        if crash_after_finality
            && capability_observation_exists(history_control, run_id, &finality_capability).await
        {
            history_control.close().await;
            std::process::exit(137);
        }
        let rendered = response.public_json().expect("render drive response");
        match rendered["kind"].as_str() {
            Some("closed") if crash_after_completion => {
                history_control.close().await;
                std::process::exit(137);
            }
            Some("closed") => {
                panic!("production application run closed before injected observation crash")
            }
            Some("advanced") => {}
            Some("waiting") => {
                panic!("production application run {run_id:?} parked before completion: {rendered}")
            }
            other => panic!("unexpected production drive outcome: {other:?}"),
        }
    }
    panic!("production application run did not reach injected observation crash");
}

async fn verify_production_projections(
    application: mfm_app::Application,
    portfolio_run_id: &RunId,
    submission_run_id: &RunId,
    portfolio: &mfm_portfolio::PortfolioConfig,
    expected_completion: &CompletedWalletNonce,
) {
    let entry_points_json = application
        .entry_points()
        .public_json()
        .expect("render production entry points");
    let portfolio_view = application
        .read_public_run(credential(), portfolio_run_id.clone())
        .await
        .expect("read resumed portfolio run");
    let portfolio_json = portfolio_view
        .public_json()
        .expect("render resumed portfolio run");
    assert_eq!(portfolio_json["status"], "closed");
    assert_eq!(portfolio_json["outcome"]["kind"], "success");
    assert_eq!(
        portfolio_json["entry_point_operation_id"],
        STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID
    );
    let portfolio_output: PortfolioPublicOutputs =
        serde_json::from_value(portfolio_json["outcome"]["value"].clone())
            .expect("decode exact production portfolio output");
    assert_eq!(
        portfolio_output.snapshot.portfolio_id,
        portfolio.portfolio_id.as_str()
    );
    assert_eq!(
        portfolio_output.snapshot.symbol_configs,
        portfolio.symbol_configs
    );
    assert_eq!(portfolio_output.snapshot.network_pins.len(), 1);
    assert_eq!(
        portfolio_output.snapshot.network_pins,
        portfolio_output.report.network_pins
    );
    let pin = &portfolio_output.snapshot.network_pins[0];
    assert_eq!(pin.network_id, "integration-network");
    let ExecutionAnchor::Evm { chain_id, block } = &pin.anchor else {
        panic!("production portfolio used a non-EVM anchor")
    };
    assert_eq!(chain_id.get(), 1);
    assert_eq!(block.number(), "100");
    assert_eq!(block.hash(), BLOCK_HASH);
    assert_eq!(portfolio_output.snapshot.wallets.len(), 1);
    let wallet = &portfolio_output.snapshot.wallets[0];
    assert_eq!(wallet.wallet_id, portfolio.wallets[0].wallet_id.as_str());
    assert_eq!(wallet.network_id, portfolio.wallets[0].network_id.as_str());
    assert_eq!(wallet.subject, portfolio.wallets[0].subject);
    assert_eq!(wallet.observations.len(), 1);
    let observation = &wallet.observations[0];
    assert_eq!(observation.wallet_id, wallet.wallet_id);
    assert_eq!(observation.symbol_id, "native.integration-network");
    assert_eq!(observation.display_symbol.as_deref(), Some("NATIVE"));
    assert_eq!(observation.network_id, "integration-network");
    assert_eq!(observation.quantity.raw_dec, "100");
    assert_eq!(observation.quantity.decimals, 18);
    assert_eq!(observation.quantity.amount_dec, "0.000000000000000100");
    assert_eq!(observation.values.len(), 1);
    assert_eq!(observation.values[0].quote.as_str(), "USD");
    assert_eq!(
        observation.values[0].priced_symbol_id,
        "native.integration-network"
    );
    assert_eq!(observation.values[0].value_dec, "0.000000000000000100");
    assert_eq!(observation.values[0].unit_price_dec, "1.00");
    assert_eq!(
        portfolio_output.report.portfolio_id,
        portfolio.portfolio_id.as_str()
    );
    assert_eq!(portfolio_output.report.wallet_summaries.len(), 1);
    assert_eq!(
        portfolio_output.report.wallet_summaries[0].wallet_id,
        wallet.wallet_id
    );
    assert_eq!(
        portfolio_output.report.wallet_summaries[0].network_id,
        wallet.network_id
    );
    assert_eq!(
        portfolio_output.report.wallet_summaries[0].totals_by_quote,
        portfolio_output.report.totals_by_quote
    );
    assert_eq!(portfolio_output.report.totals_by_quote.len(), 1);
    assert_eq!(
        portfolio_output.report.totals_by_quote[0].quote.as_str(),
        "USD"
    );
    assert_eq!(
        portfolio_output.report.totals_by_quote[0].total_value_dec,
        "0.0000000000000001"
    );

    let submission_view = application
        .read_public_run(credential(), submission_run_id.clone())
        .await
        .expect("read resumed submission run");
    let submission_json = submission_view
        .public_json()
        .expect("render resumed submission run");
    assert_eq!(submission_json["status"], "closed");
    assert_eq!(submission_json["outcome"]["kind"], "success");
    assert_eq!(
        submission_json["entry_point_operation_id"],
        EVM_SUBMIT_TRANSACTION_OPERATION_ID
    );
    let submission_output: EvmSubmissionOutput =
        serde_json::from_value(submission_json["outcome"]["value"].clone())
            .expect("decode exact production submission output");
    assert_eq!(
        submission_output.execution_disposition,
        expected_completion
            .canonical_terminal_outcome
            .execution_disposition
    );
    assert_eq!(
        submission_json["outcome"]["value"],
        serde_json::json!({"execution_disposition": "succeeded"}),
        "public result must not carry completion evidence"
    );

    let replay = application
        .replay_run(
            credential(),
            portfolio_run_id.clone(),
            ReplayRequest::Verify,
        )
        .await
        .expect("callback-free production replay");
    let replay_json = replay.public_json().expect("render replay");
    assert_eq!(replay_json["kind"], "verified");
    assert_eq!(replay_json["status"], "closed");
    let closed_drive_json = application
        .drive_once(credential(), portfolio_run_id.clone())
        .await
        .expect("drive already closed production run")
        .public_json()
        .expect("render closed drive response");
    assert_eq!(closed_drive_json["kind"], "closed");
    let trace = application
        .read_transition_trace(
            credential(),
            portfolio_run_id.clone(),
            PageRequest::new(None, Some(500)).expect("trace page request"),
        )
        .await
        .expect("production transition projection");
    assert_eq!(trace.run_id(), portfolio_run_id);
    assert!(!trace.transitions().is_empty());
    let trace_json = trace.public_json().expect("render transition projection");
    let audit = application
        .read_access_audit(
            credential(),
            portfolio_run_id.clone(),
            PageRequest::new(None, Some(500)).expect("audit page request"),
        )
        .await
        .expect("production access projection");
    assert_eq!(audit.run_id(), portfolio_run_id);
    assert!(!audit.entries().is_empty());
    let audit_json = audit.public_json().expect("render access projection");

    let (semantic_ref, semantic_bytes) =
        read_application_export(&application, portfolio_run_id, ExportKind::Semantic).await;
    let (audit_ref, audit_bytes) =
        read_application_export(&application, portfolio_run_id, ExportKind::Audit).await;
    let semantic_export = PortableRunExport::strict_decode(&semantic_bytes)
        .expect("decode semantic portable frame stream");
    let audit_export =
        PortableRunExport::strict_decode(&audit_bytes).expect("decode audit portable frame stream");
    let semantic_encoded = semantic_export.encode().expect("re-encode semantic");
    let audit_encoded = audit_export.encode().expect("re-encode audit");
    assert_eq!(semantic_encoded.as_bytes(), semantic_bytes);
    assert_eq!(audit_encoded.as_bytes(), audit_bytes);
    assert_eq!(semantic_encoded.content_ref(), &semantic_ref);
    assert_eq!(audit_encoded.content_ref(), &audit_ref);
    let semantic_frames = portable_frames(&semantic_bytes);
    let audit_frames = portable_frames(&audit_bytes);
    assert!(semantic_frames.len() > 1);
    assert!(audit_frames.len() > 1);
    assert!(
        semantic_frames.len() <= audit_frames.len(),
        "semantic export must not retain more physical frames than audit"
    );
    let semantic_seal = portable_seal(semantic_frames.last().expect("semantic seal"));
    let audit_seal = portable_seal(audit_frames.last().expect("audit seal"));
    assert_eq!(semantic_seal["kind"], "seal");
    assert_eq!(audit_seal["kind"], "seal");
    assert_eq!(semantic_seal["payload"]["kind"], "semantic");
    assert_eq!(audit_seal["payload"]["kind"], "audit");
    assert_eq!(
        semantic_seal["payload"]["version"],
        "mfm.structured-portable-run-export-stream.v2"
    );
    assert_eq!(
        audit_seal["payload"]["version"],
        "mfm.structured-portable-run-export-stream.v2"
    );
    let post_telemetry_application = application.clone();
    let router = mfm_rest_api::make_app(mfm_rest_api::AppState::new(application));
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/entry-points")
                .body(Body::empty())
                .expect("REST entry-point request"),
        )
        .await
        .expect("production REST entry-point projection");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        rest_success_data(response).await,
        entry_points_json,
        "REST entry-point response must preserve the reviewed application projection"
    );

    let portfolio_retry = admission_request(
        PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
        InvocationIdentity::new(PORTFOLIO_INVOCATION).expect("portfolio invocation identity"),
        &PortfolioSnapshotSelector::new(
            PortfolioId::new("production_portfolio").expect("portfolio target"),
        ),
    );
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs")
                .header(
                    "authorization",
                    format!("Bearer {SECRET_CREDENTIAL_CANARY}"),
                )
                .body(Body::from(portfolio_retry.as_bytes().to_vec()))
                .expect("REST admission request"),
        )
        .await
        .expect("production REST admission projection");
    assert_eq!(response.status(), StatusCode::OK);
    let admission_json = rest_success_data(response).await;
    assert_eq!(admission_json["run_id"], portfolio_run_id.as_str());
    assert_eq!(admission_json["admission"], "attached");

    let telemetry_sink = FailingTelemetrySink::default();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(telemetry_sink.clone()),
    );
    let dispatch = tracing::Dispatch::new(subscriber);
    let telemetry_guard = tracing::dispatcher::set_default(&dispatch);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{portfolio_run_id}/drive"))
                .header(
                    "authorization",
                    format!("Bearer {SECRET_CREDENTIAL_CANARY}"),
                )
                .body(Body::empty())
                .expect("REST drive request"),
        )
        .await
        .expect("production REST drive projection despite telemetry failure");
    drop(telemetry_guard);
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(rest_success_data(response).await, closed_drive_json);
    assert!(telemetry_sink.failure_count() > 0);
    assert_canaries_absent(
        "failed operational telemetry",
        &telemetry_sink.attempted_bytes(),
    );
    assert_eq!(
        post_telemetry_application
            .read_public_run(credential(), portfolio_run_id.clone())
            .await
            .expect("read run after operational telemetry failure")
            .public_json()
            .expect("render run after operational telemetry failure"),
        portfolio_json,
        "best-effort telemetry failure must not change run history or result"
    );

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/runs/{portfolio_run_id}"))
                .header(
                    "authorization",
                    format!("Bearer {SECRET_CREDENTIAL_CANARY}"),
                )
                .body(Body::empty())
                .expect("REST request"),
        )
        .await
        .expect("production REST projection");
    assert_eq!(response.status(), StatusCode::OK);
    let rest_body = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .expect("read production REST body");
    let rest: Value = serde_json::from_slice(&rest_body).expect("decode production REST body");
    assert_eq!(rest["status"], "success");
    assert_eq!(rest["data"], portfolio_json);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{portfolio_run_id}/replay?mode=verify"))
                .header(
                    "authorization",
                    format!("Bearer {SECRET_CREDENTIAL_CANARY}"),
                )
                .body(Body::empty())
                .expect("REST replay request"),
        )
        .await
        .expect("production REST replay projection");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(rest_success_data(response).await, replay_json);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/runs/{portfolio_run_id}/trace?limit=500"))
                .header(
                    "authorization",
                    format!("Bearer {SECRET_CREDENTIAL_CANARY}"),
                )
                .body(Body::empty())
                .expect("REST trace request"),
        )
        .await
        .expect("production REST trace projection");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(rest_success_data(response).await, trace_json);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/runs/{portfolio_run_id}/audit?limit=500"))
                .header(
                    "authorization",
                    format!("Bearer {SECRET_CREDENTIAL_CANARY}"),
                )
                .body(Body::empty())
                .expect("REST audit request"),
        )
        .await
        .expect("production REST audit projection");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(rest_success_data(response).await, audit_json);

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{portfolio_run_id}/exports"))
                .header(
                    "authorization",
                    format!("Bearer {SECRET_CREDENTIAL_CANARY}"),
                )
                .body(Body::from(r#"{"kind":"semantic"}"#))
                .expect("REST export request"),
        )
        .await
        .expect("production REST export projection");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some(mfm_app::PORTABLE_RUN_EXPORT_MEDIA_TYPE)
    );
    assert_eq!(
        response
            .headers()
            .get("mfm-content-digest")
            .and_then(|value| value.to_str().ok()),
        Some(semantic_ref.content_digest().as_str())
    );
    let rest_export_bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .expect("read production REST export");
    assert_canaries_absent("REST portable export", &rest_export_bytes);
    assert_eq!(rest_export_bytes, semantic_bytes);

    for (label, value) in [
        ("entry-point program projection", &entry_points_json),
        ("portfolio public result", &portfolio_json),
        ("submission public result", &submission_json),
        ("replay result", &replay_json),
        ("closed drive result", &closed_drive_json),
        ("transition trace", &trace_json),
        ("access audit", &audit_json),
    ] {
        assert_canaries_absent(
            label,
            canonical_json(value)
                .expect("canonical canary surface")
                .as_bytes(),
        );
    }
    assert_canaries_absent("semantic portable export", &semantic_bytes);
    assert_canaries_absent("audit portable export", &audit_bytes);
}

#[derive(Clone, Default)]
struct FailingTelemetrySink {
    attempted: Arc<Mutex<Vec<u8>>>,
    failures: Arc<AtomicU64>,
}

impl FailingTelemetrySink {
    fn attempted_bytes(&self) -> Vec<u8> {
        self.attempted
            .lock()
            .expect("telemetry capture lock")
            .clone()
    }

    fn failure_count(&self) -> u64 {
        self.failures.load(Ordering::Acquire)
    }
}

struct FailingTelemetryWriter {
    sink: FailingTelemetrySink,
}

impl Write for FailingTelemetryWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.sink
            .attempted
            .lock()
            .map_err(|_| io::Error::other("telemetry capture unavailable"))?
            .extend_from_slice(bytes);
        self.sink.failures.fetch_add(1, Ordering::AcqRel);
        Err(io::Error::other("telemetry sink unavailable"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("telemetry sink unavailable"))
    }
}

impl<'writer> MakeWriter<'writer> for FailingTelemetrySink {
    type Writer = FailingTelemetryWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        FailingTelemetryWriter { sink: self.clone() }
    }
}

fn assert_canaries_absent(label: &str, bytes: &[u8]) {
    for canary in [SECRET_CREDENTIAL_CANARY, PROVIDER_TEXT_CANARY] {
        assert!(
            !bytes
                .windows(canary.len())
                .any(|window| window == canary.as_bytes()),
            "{label} retained secret-bearing input"
        );
    }
}

async fn rest_success_data(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .expect("read production REST success body");
    assert_canaries_absent("REST success response", &body);
    let rendered: Value = serde_json::from_slice(&body).expect("decode production REST success");
    assert_eq!(rendered["status"], "success");
    rendered["data"].clone()
}

async fn read_application_export(
    application: &mfm_app::Application,
    run_id: &RunId,
    kind: ExportKind,
) -> (ContentRef, Vec<u8>) {
    let export = application
        .export_run(credential(), run_id.clone(), ExportRequest::new(kind))
        .await
        .expect("production portable export");
    let content_ref = export.content_ref().clone();
    let mut reader = export.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .expect("read production portable export");
    (content_ref, bytes)
}

fn portable_frames(bytes: &[u8]) -> Vec<&[u8]> {
    assert!(
        bytes.ends_with(b"\n"),
        "portable stream newline termination"
    );
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect()
}

fn portable_seal(frame: &[u8]) -> Value {
    let value: Value = serde_json::from_slice(frame).expect("portable frame JSON");
    assert_eq!(value["kind"], "seal");
    value
}

struct RuntimeAssembly {
    registry: mfm_certify::structured::QualifiedProgramRegistry,
    physical_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
    document: mfm_spec::structured::CertifiedProgramDocument,
    admission_material: StructuredAdmissionMaterial,
    request: EvmSubmissionRequest,
}

async fn assemble_runtime(
    fixture: &Fixture,
    activation: mfm_evm::WalletNonceDomainActivationAttestation,
    rpc_endpoint: &str,
    wallet_authority: Arc<dyn WalletNonceAuthority>,
    verify_registration_order: bool,
) -> RuntimeAssembly {
    let routing_policy = admission_object(
        ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
        "mfm.evm.integration.routing-policy",
        4,
    );
    let rpc_certificate = certificate("rpc-certificate", 5);
    let signer_certificate = certificate("signer-certificate", 6);
    let signer_fence_certificate = certificate("signer-fence-certificate", 7);
    let broadcast_certificate = certificate("broadcast-certificate", 8);
    let broadcast_head_object = certificate("broadcast-head", 9);
    let wallet_read_certificate = certificate("wallet-read-certificate", 10);
    let wallet_effect_certificate = certificate("wallet-effect-certificate", 11);

    let chain_binding = fixture.chain_attestation.binding().expect("chain binding");
    let mut routes = EvmRoutingCatalogBuilder::new(
        fixture.routing_catalog.chain_registry_head_ref().clone(),
        vec![fixture.chain_attestation.clone()],
    )
    .expect("routing catalog builder");
    let route = routes
        .insert(
            fixture.routing_descriptor.clone(),
            EvmRpcEndpoint::new(rpc_endpoint).expect("loopback endpoint"),
            None,
        )
        .expect("insert exact route");
    assert_eq!(
        route,
        fixture
            .routing_descriptor
            .generation_ref()
            .expect("qualified route reference")
    );
    let route_generation_ref = fixture.route_generation_ref.clone();
    let request = fixture.submission_request(activation, route_generation_ref.clone());
    let transport = Arc::new(
        EvmJsonRpcTransport::new(routes.build().expect("routing catalog")).expect("EVM transport"),
    );
    let signer = deterministic_signer(
        fixture.sender,
        &signer_certificate,
        &signer_fence_certificate,
        &rpc_certificate,
    )
    .await;
    let rpc_target_ref = route_generation_ref
        .to_content_ref()
        .expect("RPC target reference");
    let signer_target_ref = signer.binding().durable_generation_ref().clone();
    let broadcast_target_ref = broadcast_head_object.content_ref.clone();
    let wallet_target_ref = wallet_authority
        .domain_activation_attestation()
        .initial_store_incarnation_ref
        .to_content_ref()
        .expect("wallet target reference");
    let broadcast_lineage_head = BroadcastLineageHead {
        lineage_id: "mfm.evm.integration/broadcast-lineage".to_owned(),
        generation: 1,
        public_lineage_head_ref: EvmWalletReference::from_content_ref(
            broadcast_head_object.content_ref.clone(),
        ),
    };
    let live = Arc::new(
        EvmStructuredLiveBindings::new(
            transport,
            chain_binding,
            route_generation_ref.clone(),
            routing_policy.content_ref.clone(),
            fixture.semantic_signer_id.clone(),
            fixture.sender,
            signer,
            single_release_history(
                &routing_policy.content_ref,
                rpc_target_ref,
                rpc_certificate.clone(),
            ),
            single_release_history(
                &routing_policy.content_ref,
                signer_target_ref,
                signer_certificate.clone(),
            ),
            single_release_history(
                &routing_policy.content_ref,
                broadcast_target_ref,
                broadcast_certificate.clone(),
            ),
            broadcast_lineage_head,
            broadcast_head_object,
        )
        .expect("qualify structured EVM bindings"),
    );

    let wallet = Arc::new(
        EvmStructuredWalletBindings::new(
            wallet_authority,
            routing_policy.content_ref.clone(),
            single_release_history(
                &routing_policy.content_ref,
                wallet_target_ref.clone(),
                wallet_read_certificate.clone(),
            ),
            single_release_history(
                &routing_policy.content_ref,
                wallet_target_ref,
                wallet_effect_certificate.clone(),
            ),
        )
        .expect("qualify structured wallet bindings"),
    );

    let operation_id = operation_id();
    let authored = structured_evm_submission_entry_program(
        operation_id.clone(),
        stable("mfm.evm.integration/structured-submit-scope"),
    )
    .expect("abstract EVM entry program");
    let qualified = qualified_submission_registry(
        Arc::clone(&live),
        Arc::clone(&wallet),
        authored.clone(),
        false,
    );
    let certified = qualified
        .certifier(&operation_id)
        .expect("entry-point certifier")
        .certify(authored.clone())
        .expect("certify EVM program");
    if verify_registration_order {
        let reverse = qualified_submission_registry(live, wallet, authored.clone(), true);
        let reverse_certified = reverse
            .certifier(&operation_id)
            .expect("reverse-order entry-point certifier")
            .certify(authored)
            .expect("certify reverse-order EVM program");
        assert_eq!(certified.document(), reverse_certified.document());
        assert_eq!(certified.reference(), reverse_certified.reference());
    }
    assert_eq!(
        certified
            .reference()
            .expect("certified EVM program reference")
            .content_digest()
            .as_str(),
        "content:sha256-v1:aac09b736b168ee1c0911a38ba0b2450da2cff4567686b09d82e87769ec6a7d8",
        "the production EVM submission program must retain its exact certification identity"
    );
    assert_eq!(certified.authored().root.declarations.len(), 1);
    assert_eq!(
        certified
            .expansion_proof()
            .substitution_trace
            .iter()
            .filter(|entry| entry.stage == ExpansionStage::ChildSubstitution)
            .count(),
        1 + EVM_WALLET_REPLACEMENT_LIMIT
    );
    let expanded_bytes = certified
        .expanded()
        .canonical_json()
        .expect("expanded canonical bytes");
    assert!(expanded_bytes.as_bytes().len() < MAX_RETAINED_DOCUMENT_BYTES);
    assert_eq!(
        certified
            .expanded()
            .canonical_json()
            .expect("repeat expanded bytes"),
        expanded_bytes
    );
    let decoded: ExpandedStructuredProgram =
        serde_json::from_slice(expanded_bytes.as_bytes()).expect("decode normalized expansion");
    assert_eq!(&decoded, certified.expanded());
    assert!(
        certified
            .document()
            .root
            .canonical_json()
            .expect("root canonical bytes")
            .as_bytes()
            .len()
            < MAX_RETAINED_DOCUMENT_BYTES
    );
    assert!(certified
        .document()
        .component_closure
        .iter()
        .all(|component| {
            component
                .value
                .canonical_json()
                .expect("component canonical bytes")
                .as_bytes()
                .len()
                < MAX_RETAINED_DOCUMENT_BYTES
        }));

    let document = certified.into_document();
    let broadcast_resource_ref = EvmBroadcastResource::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .expect("broadcast resource contract");
    let wallet_resource_ref = WalletNonceAuthorityResource::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .expect("wallet resource contract");
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> =
        Arc::new(ExactPhysicalBindingVerifier::new(
            &document,
            routing_policy.content_ref.clone(),
            [
                (rpc_certificate, None),
                (signer_certificate, None),
                (broadcast_certificate, Some(broadcast_resource_ref.clone())),
                (wallet_read_certificate, None),
                (wallet_effect_certificate, Some(wallet_resource_ref.clone())),
            ],
        ));
    let admission_material = StructuredAdmissionMaterial::new(
        admission_object(
            ADMISSION_CONFIGURATION_OBJECT_TYPE,
            "mfm.evm.integration.configuration",
            1,
        ),
        admission_object(
            ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
            "mfm.evm.integration.context",
            2,
        ),
        PriorRunFactSourceManifest::new(Vec::new())
            .and_then(|manifest| manifest.to_history_object())
            .expect("prior-run source manifest"),
        routing_policy,
        vec![broadcast_resource_ref, wallet_resource_ref],
    )
    .expect("admission material");
    RuntimeAssembly {
        registry: qualified,
        physical_verifier,
        document,
        admission_material,
        request,
    }
}

fn qualified_submission_registry(
    live: Arc<EvmStructuredLiveBindings>,
    wallet: Arc<EvmStructuredWalletBindings>,
    authored: mfm_spec::structured::AuthoredStructuredProgram,
    reverse_registration_order: bool,
) -> mfm_certify::structured::QualifiedProgramRegistry {
    let mut registry = ProgramRegistryBuilder::new();
    let executable_identity_ref = registry
        .register_executable_identity(SecretFreeExecutableIdentity {
            executable_id: stable("mfm.evm.integration/structured-executable"),
        })
        .expect("register executable identity");
    let qualification_artifact_ref = registry
        .register_qualification_artifact(SecretFreeQualificationArtifact {
            qualification_id: stable("mfm.evm.integration/structured-qualification"),
        })
        .expect("register qualification artifact");
    let qualification =
        EvmSubmissionProcessQualification::new(executable_identity_ref, qualification_artifact_ref);
    register_evm_submission_process(&mut registry, &qualification)
        .expect("register deterministic EVM process");

    let operation_id = operation_id();
    if reverse_registration_order {
        registry
            .register_entry_point(operation_id.clone(), authored, submission_profile())
            .expect("register reverse-order EVM entry point");
        register_evm_wallet_authority_bindings(&mut registry, &qualification, wallet)
            .expect("register reverse-order wallet authority bindings");
        register_evm_live_submission_bindings(&mut registry, &qualification, live)
            .expect("register reverse-order transport and signer bindings");
    } else {
        register_evm_live_submission_bindings(&mut registry, &qualification, live)
            .expect("register transport and signer bindings");
        register_evm_wallet_authority_bindings(&mut registry, &qualification, wallet)
            .expect("register wallet authority bindings");
        registry
            .register_entry_point(operation_id.clone(), authored, submission_profile())
            .expect("register EVM entry point");
    }
    registry
        .build(std::slice::from_ref(&operation_id))
        .expect("build process registry")
}

fn submission_profile() -> StructuredExpansionProfile {
    StructuredExpansionProfile {
        policies: Vec::new(),
        max_occurrences: 4096,
        max_declarations: 4096,
        max_lanes: 1,
        max_fan_out_depth: 1,
        max_branch_depth: 16,
    }
}

async fn deterministic_signer(
    sender: Address,
    generation: &HistoryObject,
    fence: &HistoryObject,
    direct_submit_exclusion: &HistoryObject,
) -> QualifiedReadSigningProvider {
    let key = SigningKey::from_slice(&INTEGRATION_SIGNING_KEY).expect("fixed test signing key");
    assert_eq!(signing_address(&key), sender);
    let public_key = PublicKeyBytes::new(
        key.verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
    .expect("compressed test signer public key");
    let binding = VerifiedGenerationGuardedSignerBinding::verify(
        SignerRef::new("structured-integration-wallet").expect("signer ref"),
        "mfm.evm.integration-signer-provider",
        SigningAlgorithmId::new(SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID)
            .expect("signing algorithm"),
        SigningProfileId::new(SECP256K1_RFC6979_LOW_S_PROFILE_ID).expect("signing profile"),
        PublicSigningIdentity::new(
            SigningAlgorithmId::new(SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID)
                .expect("public signing algorithm"),
            Some(public_key),
            Some(format!("{sender:#x}")),
        )
        .expect("public signer identity"),
        generation.content_ref.clone(),
        fence.content_ref.clone(),
        direct_submit_exclusion.content_ref.clone(),
    )
    .expect("generation-guarded signer binding");
    let provider: Arc<dyn GenerationGuardedDeterministicSigningProvider> =
        Arc::new(DeterministicTestSigner { binding, key });
    QualifiedReadSigningProvider::try_qualify(provider)
        .await
        .expect("qualify deterministic integration signer")
}

struct CurrentSigningGuard;

impl SigningGenerationGuard for CurrentSigningGuard {
    fn is_read_attestation_eligible(&self) -> bool {
        true
    }

    fn verify_current_and_exclusive<'a>(
        &'a self,
        _binding: &'a VerifiedGenerationGuardedSignerBinding,
    ) -> SigningGenerationGuardFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

async fn qualified_production_signer(
    fixture: &Fixture,
    generation: &HistoryObject,
    fence: &HistoryObject,
    direct_submit_exclusion: &HistoryObject,
) -> (mfm_keystore::QualifiedKeystoreSigner, tempfile::TempDir) {
    let directory = tempfile::tempdir().expect("create integration signer directory");
    let keystore_path = directory.path().join("wallet.json");
    let unlock_path = directory.path().join("unlock");
    let config = KeystoreConfig::insecure_integration_test();
    let mut keystore = Keystore::new_with_config(&keystore_path, config.clone())
        .expect("create integration keystore");
    keystore
        .unlock(KEYSTORE_PASSWORD)
        .expect("unlock integration keystore");
    let private_key_hex = Zeroizing::new(format!("0x{}", hex::encode(INTEGRATION_SIGNING_KEY)));
    let entry_id = keystore
        .import_private_key(Some("production-wallet".to_owned()), &private_key_hex)
        .expect("import integration signing key");
    std::fs::write(&unlock_path, KEYSTORE_PASSWORD).expect("write integration unlock source");
    let key =
        SigningKey::from_slice(&INTEGRATION_SIGNING_KEY).expect("fixed integration signing key");
    let public_key = PublicKeyBytes::new(
        key.verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
    .expect("compressed integration public key");
    let binding = VerifiedGenerationGuardedSignerBinding::verify(
        SignerRef::new("structured-integration-wallet").expect("signer ref"),
        mfm_keystore::KEYSTORE_SIGNING_IMPLEMENTATION_ID,
        SigningAlgorithmId::new(SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID)
            .expect("signing algorithm"),
        SigningProfileId::new(SECP256K1_RFC6979_LOW_S_PROFILE_ID).expect("signing profile"),
        PublicSigningIdentity::new(
            SigningAlgorithmId::new(SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID)
                .expect("public signing algorithm"),
            Some(public_key),
            Some(format!("{:#x}", fixture.sender)),
        )
        .expect("complete public signer identity"),
        generation.content_ref.clone(),
        fence.content_ref.clone(),
        direct_submit_exclusion.content_ref.clone(),
    )
    .expect("keystore signer binding");
    let provider = KeystoreSignerProvider::new_with_config(
        binding,
        entry_id,
        keystore_path,
        unlock_path,
        Arc::new(CurrentSigningGuard),
        config,
    )
    .expect("keystore signer provider");
    let signer_contract_ref = EvmCandidateSigner::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .expect("candidate signer contract");
    let qualified = provider
        .qualify(fixture.semantic_signer_id.clone(), signer_contract_ref)
        .await
        .expect("qualify keystore signer");
    (qualified, directory)
}

#[derive(Clone)]
struct DeterministicTestSigner {
    binding: VerifiedGenerationGuardedSignerBinding,
    key: SigningKey,
}

impl GenerationGuardedDeterministicSigningProvider for DeterministicTestSigner {
    fn is_read_attestation_eligible(&self) -> bool {
        true
    }

    fn binding(&self) -> &VerifiedGenerationGuardedSignerBinding {
        &self.binding
    }

    fn verify_read_attestation_qualification<'a>(
        &'a self,
        expected_binding: &'a VerifiedGenerationGuardedSignerBinding,
    ) -> ReadAttestationQualificationFuture<'a> {
        let result = if expected_binding == &self.binding {
            Ok(())
        } else {
            Err(SigningError::Provider {
                reason: SigningProviderError::BindingMismatch,
            })
        };
        Box::pin(async move { result })
    }

    fn sign_guarded<'a>(
        &'a self,
        expected_generation_ref: &'a ContentRef,
        request: &'a SigningRequest,
    ) -> SigningFuture<'a> {
        let result = if expected_generation_ref != self.binding.durable_generation_ref() {
            Err(SigningError::Provider {
                reason: SigningProviderError::GenerationMismatch,
            })
        } else {
            self.binding.verify_request(request).and_then(|()| {
                let (signature, recovery_id) = self
                    .key
                    .sign_prehash_recoverable(request.digest().as_ref())
                    .map_err(|_| SigningError::Provider {
                        reason: SigningProviderError::Failed,
                    })?;
                let signature = signature.to_bytes();
                let signature = PrimitiveSignature::from_scalars_and_parity(
                    B256::from_slice(&signature[..32]),
                    B256::from_slice(&signature[32..]),
                    recovery_id.is_y_odd(),
                )
                .normalized_s();
                SigningResult::for_request(
                    request,
                    self.binding.expected_public_identity().clone(),
                    SignatureBytes::new(signature.as_bytes().to_vec())?,
                )
            })
        };
        Box::pin(async move { result })
    }
}

fn signing_address(key: &SigningKey) -> Address {
    let encoded = key.verifying_key().to_encoded_point(false);
    let digest = keccak256(&encoded.as_bytes()[1..]);
    Address::from_slice(&digest[12..])
}

struct Fixture {
    common_ref: EvmWalletReference,
    registry_lineage_ref: EvmWalletReference,
    provider_id: StableId,
    provider_fence_lineage_ref: EvmWalletReference,
    provider_fence_head_ref: EvmWalletReference,
    chain_attestation: ChainInstanceRegistryAttestation,
    routing_descriptor: EvmRoutingGenerationDescriptor,
    routing_catalog: EvmRoutingCatalogDescriptor,
    route_generation_ref: EvmWalletReference,
    nonce_domain: WalletNonceDomain,
    incarnation: WalletNonceStoreIncarnation,
    activation_record: WalletNonceDomainActivationRecord,
    current_public_head: HistoryObject,
    semantic_signer_id: StableId,
    tenant: TenantScopeId,
    principal: StableId,
    sender: Address,
}

impl Fixture {
    fn new() -> Self {
        let common_ref = evm_wallet_nonce_policy_ref().expect("wallet policy reference");
        let registry_lineage_ref = authority_reference("activation-registry-lineage", 0xc1);
        let provider_fence_lineage_ref = authority_reference("provider-fence-lineage", 0xc2);
        let chain_registry_lineage_ref = authority_reference("chain-registry-lineage", 0xc3);
        let chain_registry_head_ref = authority_reference("chain-registry-head", 0xc4);
        let chain_registry_issuance_ref = authority_reference("chain-registry-issuance", 0xc5);
        let route_membership_ref = authority_reference("route-membership", 0xc6);
        let chain_declaration = ChainInstanceDeclaration::new(
            chain_registry_lineage_ref,
            stable("mfm.evm.integration/chain-instance"),
            1,
            B256::repeat_byte(0x11),
            U256::from(90_u64),
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
        let routing_descriptor = EvmRoutingGenerationDescriptor::new(
            "integration-network",
            "mfm.evm.integration.source",
            stable("mfm.evm.integration/route-generation"),
            route_membership_ref.clone(),
            chain_binding,
        )
        .expect("routing descriptor");
        let route_generation = routing_descriptor
            .generation_ref()
            .expect("route generation");
        let route_generation_ref = EvmWalletReference::from_content_ref(
            route_generation
                .to_content_ref()
                .expect("route generation reference"),
        );
        let routing_catalog = EvmRoutingCatalogDescriptor::new(
            chain_registry_head_ref,
            vec![chain_attestation.clone()],
            vec![routing_descriptor.clone()],
        )
        .expect("routing catalog");
        let key = SigningKey::from_slice(&INTEGRATION_SIGNING_KEY).expect("fixed test signing key");
        let sender = signing_address(&key);
        let public_key = PublicKeyBytes::new(
            key.verifying_key()
                .to_encoded_point(true)
                .as_bytes()
                .to_vec(),
        )
        .expect("compressed signing public key");
        let semantic_signer_id = derive_evm_semantic_signer_id(
            &PublicSigningIdentity::new(
                SigningAlgorithmId::new(SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID)
                    .expect("signing algorithm"),
                Some(public_key),
                Some(format!("{sender:#x}")),
            )
            .expect("complete signer identity"),
        )
        .expect("semantic signer identity");
        let nonce_domain =
            derive_wallet_nonce_domain(chain_lineage, sender).expect("wallet nonce domain");
        let incarnation = WalletNonceStoreIncarnation {
            wallet_nonce_store_lineage_id: "mfm.evm.integration/wallet-lineage".to_owned(),
            writer_epoch: 1,
            physical_target_instance_id: "mfm.evm.integration/wallet-target".to_owned(),
            non_exportable_target_public_key_ref: common_ref.clone(),
            target_attestation_contract_ref: common_ref.clone(),
        };
        let inventory = mfm_journal::structured::domain_content_digest(
            "mfm.evm.integration/sender-path-inventory.v1",
            &nonce_domain,
        )
        .expect("sender-path inventory");
        let activation_record = WalletNonceDomainActivationRecord {
            activation_contract_ref: common_ref.clone(),
            qualified_activation_registry_lineage_ref: registry_lineage_ref.clone(),
            wallet_nonce_store_lineage_id: incarnation.wallet_nonce_store_lineage_id.clone(),
            initial_store_incarnation_ref: canonical_wallet_reference(&incarnation)
                .expect("initial incarnation reference"),
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
            finalized_block_number: "90".to_owned(),
            finalized_block_hash: format!("{:#x}", B256::repeat_byte(0x12)),
            qualified_observation_proof_ref: common_ref.clone(),
            exhaustive_sender_path_inventory_digest: inventory.as_str().to_owned(),
            exclusive_current_control: ExclusiveCurrentControl::EveryPriorWriterSignerRelayerOperatorStaleDeploymentAndDirectSubmitPathFenced,
            prior_effect_disposition: PriorEffectDisposition::NoUnresolvedPossibleEntry,
            prior_resource_disposition:
                PriorResourceDisposition::EveryPriorAllocationAndSubmittedCandidateTerminal,
            new_idempotency_epoch: "mfm.evm.integration/idempotency-epoch".to_owned(),
        };
        let current_public_head = certificate("wallet-current-head", 12);
        Self {
            common_ref,
            registry_lineage_ref,
            provider_id: stable("mfm.evm.integration/wallet-authority-provider"),
            provider_fence_lineage_ref,
            provider_fence_head_ref: EvmWalletReference::from_content_ref(
                current_public_head.content_ref.clone(),
            ),
            chain_attestation,
            routing_descriptor,
            routing_catalog,
            route_generation_ref,
            nonce_domain,
            incarnation,
            activation_record,
            current_public_head,
            semantic_signer_id,
            tenant: TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000051")
                .expect("tenant"),
            principal: stable("mfm.evm.integration/principal"),
            sender,
        }
    }

    fn submission_configuration(
        &self,
        activation: mfm_evm::WalletNonceDomainActivationAttestation,
        route_generation_ref: EvmWalletReference,
    ) -> EvmSubmissionConfiguration {
        let transaction_template = EvmWalletTransactionTemplate::new(
            EvmTransactionTarget::new("mfm.evm.integration-recipient").expect("transaction target"),
            EvmWalletTransactionAction::call(RECIPIENT),
            U256::ZERO,
            [],
            Vec::new(),
            U256::from(21_000_u64),
        )
        .expect("transaction template");
        let transaction_intent = EvmTransactionIntent::new(
            self.chain_attestation.binding().expect("chain binding"),
            self.nonce_domain.clone(),
            transaction_template,
            self.semantic_signer_id.clone(),
            evm_deterministic_signing_profile_ref().expect("signing profile"),
            EvmWalletReference::from_content_ref(
                BroadcastExactCandidateCapability::contract()
                    .and_then(|contract| contract.content_ref().map_err(Into::into))
                    .expect("broadcast contract"),
            ),
            evm_wallet_assurance_policy_ref().expect("assurance policy"),
        )
        .expect("transaction intent");
        let candidate_family = EvmCandidateFamily::new(
            &transaction_intent,
            vec![
                EvmWalletFeeCandidate::new(U256::from(20_u64), U256::from(2_u64))
                    .expect("fee candidate"),
            ],
        )
        .expect("candidate family");
        EvmSubmissionConfiguration::new(
            activation,
            self.activation_record.issuer_namespace_contract_ref.clone(),
            route_generation_ref,
            transaction_intent,
            candidate_family,
            1,
        )
        .expect("submission configuration")
    }

    fn submission_request(
        &self,
        activation: mfm_evm::WalletNonceDomainActivationAttestation,
        route_generation_ref: EvmWalletReference,
    ) -> EvmSubmissionRequest {
        EvmSubmissionRequest::from_authorized(
            self.submission_configuration(activation, route_generation_ref),
            self.tenant.clone(),
            self.principal.clone(),
            EvmCallerSubmissionToken::new(SUBMISSION_TOKEN).expect("submission caller token"),
        )
        .expect("authorized submission request")
    }

    fn cross_chain_submission_configuration(&self) -> EvmSubmissionConfiguration {
        let declaration = ChainInstanceDeclaration::new(
            self.common_ref.clone(),
            stable("mfm.evm.integration/cross-chain-instance"),
            2,
            B256::repeat_byte(0x21),
            U256::from(90_u64),
            B256::repeat_byte(0x22),
        )
        .expect("cross-chain declaration");
        let attestation = ChainInstanceRegistryAttestation::new(
            declaration,
            self.common_ref.clone(),
            self.common_ref.clone(),
        )
        .expect("cross-chain attestation");
        let binding = attestation.binding().expect("cross-chain binding");
        let lineage = derive_evm_chain_lineage_id(binding.qualified_chain_instance_id())
            .expect("cross-chain lineage");
        let nonce_domain =
            derive_wallet_nonce_domain(lineage, self.sender).expect("cross-chain nonce domain");
        let route = EvmRoutingGenerationDescriptor::new(
            "integration-network-cross-chain",
            "mfm.evm.integration.cross-chain-source",
            stable("mfm.evm.integration/cross-chain-route-generation"),
            self.common_ref.clone(),
            binding.clone(),
        )
        .expect("cross-chain route descriptor");
        let mut activation_record = self.activation_record.clone();
        activation_record.wallet_nonce_domain = nonce_domain.clone();
        activation_record.chain_instance_attestation = attestation;
        activation_record.initial_route_generation_ref = route
            .generation_ref()
            .expect("cross-chain route generation");
        activation_record.exhaustive_sender_path_inventory_digest =
            mfm_journal::structured::domain_content_digest(
                "mfm.evm.integration/sender-path-inventory.v1",
                &nonce_domain,
            )
            .expect("cross-chain sender inventory")
            .as_str()
            .to_owned();
        activation_record
            .validate()
            .expect("cross-chain activation record");
        let activation = WalletNonceDomainActivationAttestation {
            activation_record_ref: canonical_wallet_reference(&activation_record)
                .expect("cross-chain activation record reference"),
            registry_issuance_ref: self.common_ref.clone(),
            activation_registry_lineage_ref: activation_record
                .qualified_activation_registry_lineage_ref
                .clone(),
            initial_store_incarnation_ref: activation_record.initial_store_incarnation_ref.clone(),
            current_schema_record: activation_record,
        };
        activation
            .validate()
            .expect("cross-chain activation attestation");
        let template = EvmWalletTransactionTemplate::new(
            EvmTransactionTarget::new("mfm.evm.integration-cross-chain-recipient")
                .expect("cross-chain target"),
            EvmWalletTransactionAction::call(RECIPIENT),
            U256::ZERO,
            [],
            Vec::new(),
            U256::from(21_000_u64),
        )
        .expect("cross-chain transaction template");
        let intent = EvmTransactionIntent::new(
            binding,
            nonce_domain,
            template,
            self.semantic_signer_id.clone(),
            self.common_ref.clone(),
            self.common_ref.clone(),
            evm_wallet_assurance_policy_ref().expect("assurance policy"),
        )
        .expect("cross-chain transaction intent");
        let family = EvmCandidateFamily::new(
            &intent,
            vec![
                EvmWalletFeeCandidate::new(U256::from(20_u64), U256::from(2_u64))
                    .expect("cross-chain fee candidate"),
            ],
        )
        .expect("cross-chain candidate family");
        EvmSubmissionConfiguration::new(
            activation,
            self.activation_record.issuer_namespace_contract_ref.clone(),
            self.route_generation_ref.clone(),
            intent,
            family,
            1,
        )
        .expect("structurally self-consistent cross-chain configuration")
    }

    fn cross_chain_submission_request(&self) -> EvmSubmissionRequest {
        EvmSubmissionRequest::from_authorized(
            self.cross_chain_submission_configuration(),
            self.tenant.clone(),
            self.principal.clone(),
            EvmCallerSubmissionToken::new(CROSS_CHAIN_SUBMISSION_TOKEN)
                .expect("cross-chain caller token"),
        )
        .expect("authorized cross-chain submission request")
    }
}

struct ExactPhysicalBindingVerifier {
    routing_policy_ref: ContentRef,
    implementation_entries: BTreeSet<(StructuredComponentKind, ContentRef, ContentRef)>,
    certificate_lineages: BTreeMap<ContentRef, Option<ContentRef>>,
}

impl mfm_authority_seal::PhysicalBindingVerifierSeal for ExactPhysicalBindingVerifier {}

impl ExactPhysicalBindingVerifier {
    fn new(
        document: &mfm_spec::structured::CertifiedProgramDocument,
        routing_policy_ref: ContentRef,
        certificates: impl IntoIterator<Item = (HistoryObject, Option<ContentRef>)>,
    ) -> Self {
        let manifest_ref = &document
            .root
            .components
            .secret_free_implementation_manifest_closure_ref;
        let manifest_object = document
            .component_closure
            .iter()
            .find(|object| &object.content_ref == manifest_ref)
            .expect("implementation manifest object");
        let manifest: SecretFreeImplementationManifest =
            serde_json::from_value(manifest_object.value.as_json().clone())
                .expect("decode implementation manifest");
        let implementation_entries = manifest
            .entries
            .into_iter()
            .map(|entry| {
                (
                    entry.component_kind,
                    entry.semantic_contract_ref,
                    entry.implementation_contract_ref,
                )
            })
            .collect();
        let certificate_lineages = certificates
            .into_iter()
            .map(|(certificate, lineage)| {
                certificate.validate().expect("valid public certificate");
                (certificate.content_ref, lineage)
            })
            .collect();
        Self {
            routing_policy_ref,
            implementation_entries,
            certificate_lineages,
        }
    }
}

impl PublicPhysicalBindingVerifier for ExactPhysicalBindingVerifier {
    fn verify_authorization(
        &self,
        context: &PhysicalBindingAuthorization<'_>,
        certificate: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        let expected_lineage = self
            .certificate_lineages
            .get(&certificate.content_ref)
            .ok_or(StructuredStoreError::Certification)?;
        let capability = (
            StructuredComponentKind::Capability,
            context.capability_contract_ref.clone(),
            context.capability_implementation_ref.clone(),
        );
        let adapter = (
            StructuredComponentKind::Adapter,
            context.adapter_contract_ref.clone(),
            context.adapter_implementation_ref.clone(),
        );
        if certificate.validate().is_ok()
            && context.admitted_routing_policy_ref == &self.routing_policy_ref
            && context.minimum_lineage_head_ref.is_none()
            && context.stable_resource_lineage_contract_ref == expected_lineage.as_ref()
            && self.implementation_entries.contains(&capability)
            && self.implementation_entries.contains(&adapter)
            && match context.access_kind {
                AccessKind::Read => expected_lineage.is_none(),
                AccessKind::Effect => expected_lineage.is_some(),
            }
        {
            Ok(())
        } else {
            Err(StructuredStoreError::Certification)
        }
    }

    fn verify_supersession(
        &self,
        _context: &PhysicalBindingSupersession<'_>,
        _public_lineage_head: &HistoryObject,
        _evidence: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        Err(StructuredStoreError::Certification)
    }
}

struct LoopbackRpc {
    endpoint: String,
    state: Arc<LoopbackRpcState>,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

struct LoopbackRpcState {
    sender: Address,
    finality_ready: AtomicBool,
    calls: Mutex<Vec<String>>,
    transaction_hash: Mutex<Option<String>>,
    /// Accepts the next broadcast and then answers ambiguously, which is the
    /// production shape of a provider whose response is lost after entry.
    ambiguous_broadcasts: AtomicUsize,
    /// Distinct signed transactions the provider ever accepted.
    accepted_transactions: Mutex<BTreeSet<String>>,
}

impl LoopbackRpc {
    async fn start(sender: Address) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback RPC");
        let address = listener.local_addr().expect("loopback address");
        let state = Arc::new(LoopbackRpcState {
            sender,
            finality_ready: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            transaction_hash: Mutex::new(None),
            ambiguous_broadcasts: AtomicUsize::new(0),
            accepted_transactions: Mutex::new(BTreeSet::new()),
        });
        let provider_path = format!("/{PROVIDER_TEXT_CANARY}");
        let app = Router::new()
            .route(&provider_path, post(loopback_rpc))
            .with_state(Arc::clone(&state));
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve loopback RPC");
        });
        Self {
            endpoint: format!("http://{address}{provider_path}"),
            state,
            shutdown: Some(shutdown),
            task,
        }
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn enable_finality(&self) {
        self.state.finality_ready.store(true, Ordering::SeqCst);
    }

    /// Makes the next `count` broadcasts enter and then answer ambiguously.
    fn answer_next_broadcasts_ambiguously(&self, count: usize) {
        self.state
            .ambiguous_broadcasts
            .store(count, Ordering::SeqCst);
    }

    /// Returns the distinct signed transactions the provider ever accepted.
    fn accepted_transaction_count(&self) -> usize {
        self.state
            .accepted_transactions
            .lock()
            .expect("accepted transaction lock")
            .len()
    }

    fn operation_count(&self, method: &str) -> usize {
        self.state
            .calls
            .lock()
            .expect("RPC call lock")
            .iter()
            .filter(|candidate| candidate.as_str() == method)
            .count()
    }

    async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await.expect("join loopback RPC");
    }
}

async fn loopback_rpc(
    State(state): State<Arc<LoopbackRpcState>>,
    Json(request): Json<Value>,
) -> Json<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::from(1));
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = request
        .get("params")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    state
        .calls
        .lock()
        .expect("RPC call lock")
        .push(method.to_owned());
    if method == "mfm_qualifyRpcInventory" {
        let Some(params) = request.get("params").and_then(Value::as_object) else {
            return Json(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"invalid params"}}),
            );
        };
        let ordinal = params
            .get("ordinal")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok());
        let route_generation_ref = params
            .get("route_generation_ref")
            .cloned()
            .and_then(|value| serde_json::from_value::<ContentRef>(value).ok());
        let strings = [
            "target_identity",
            "assembly_lease",
            "checkpoint",
            "route_challenge",
            "finish_authorization_commitment",
        ]
        .map(|name| params.get(name).and_then(Value::as_str));
        let (
            Some(ordinal),
            Some(route_generation_ref),
            [Some(target_identity), Some(assembly_lease), Some(checkpoint), Some(route_challenge), Some(finish_commitment)],
        ) = (ordinal, route_generation_ref, strings)
        else {
            return Json(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"invalid params"}}),
            );
        };
        if params.get("protocol").and_then(Value::as_str)
            != Some("mfm.evm.rpc-inventory-qualification.v1")
            || target_identity != hex::encode(RPC_TARGET_IDENTITY)
        {
            return Json(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"invalid params"}}),
            );
        }
        let message = serde_json::to_vec(&(
            ordinal,
            &route_generation_ref,
            target_identity,
            assembly_lease,
            checkpoint,
            route_challenge,
            finish_commitment,
        ))
        .expect("encode RPC inventory proof message");
        let key = hmac::Key::new(hmac::HMAC_SHA256, &RPC_PROOF_KEY);
        let proof = hex::encode(hmac::sign(&key, &message).as_ref());
        return Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocol": "mfm.evm.rpc-inventory-qualification.v1",
                "ordinal": ordinal,
                "route_generation_ref": route_generation_ref,
                "target_identity": target_identity,
                "assembly_lease": assembly_lease,
                "checkpoint": checkpoint,
                "route_challenge": route_challenge,
                "finish_authorization_commitment": finish_commitment,
                "proof": proof
            }
        }));
    }
    let result = match method {
        "eth_chainId" if params.is_empty() => Some(Value::String("0x1".to_owned())),
        "eth_getTransactionCount"
            if params
                == [
                    Value::String(format!("{:#x}", state.sender)),
                    Value::String("pending".to_owned()),
                ] =>
        {
            Some(Value::String("0x7".to_owned()))
        }
        "eth_sendRawTransaction" => params
            .first()
            .and_then(Value::as_str)
            .and_then(|value| value.strip_prefix("0x"))
            .and_then(|encoded| {
                let mut signed = Zeroizing::new(hex::decode(encoded).ok()?);
                let hash = format!("{:#x}", keccak256(signed.as_slice()));
                signed.fill(0);
                let mut retained = state
                    .transaction_hash
                    .lock()
                    .expect("transaction hash lock");
                if retained.as_ref().is_some_and(|known| known != &hash) {
                    return None;
                }
                *retained = Some(hash.clone());
                // The transaction enters whether or not the caller learns so.
                state
                    .accepted_transactions
                    .lock()
                    .expect("accepted transaction lock")
                    .insert(hash.clone());
                if state
                    .ambiguous_broadcasts
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok()
                {
                    // Entered, then answered ambiguously: the adapter must not
                    // conclude non-entry from this.
                    return None;
                }
                Some(Value::String(hash))
            }),
        "eth_getTransactionByHash" => {
            let hash = exact_requested_hash(&state, &params);
            if !state.finality_ready.load(Ordering::SeqCst) {
                hash.map(|_| Value::Null)
            } else {
                hash.map(|hash| {
                    json!({
                        "accessList": [],
                        "blockHash": BLOCK_HASH,
                        "blockNumber": BLOCK_NUMBER,
                        "chainId": "0x1",
                        "from": format!("{:#x}", state.sender),
                        "gas": "0x5208",
                        "hash": hash,
                        "input": "0x",
                        "maxFeePerGas": "0x14",
                        "maxPriorityFeePerGas": "0x2",
                        "nonce": "0x7",
                        "to": format!("{RECIPIENT:#x}"),
                        "transactionIndex": "0x0",
                        "type": "0x2",
                        "value": "0x0"
                    })
                })
            }
        }
        "eth_getTransactionReceipt" => {
            let hash = exact_requested_hash(&state, &params);
            if !state.finality_ready.load(Ordering::SeqCst) {
                hash.map(|_| Value::Null)
            } else {
                hash.map(|hash| {
                    json!({
                        "blockHash": BLOCK_HASH,
                        "blockNumber": BLOCK_NUMBER,
                        "contractAddress": null,
                        "cumulativeGasUsed": "0x5208",
                        "from": format!("{:#x}", state.sender),
                        "gasUsed": "0x5208",
                        "logs": [],
                        "status": "0x1",
                        "to": format!("{RECIPIENT:#x}"),
                        "transactionHash": hash,
                        "transactionIndex": "0x0",
                        "type": "0x2"
                    })
                })
            }
        }
        "eth_getBlockByNumber" => match params.as_slice() {
            [Value::String(selector), Value::Bool(false)] if selector == "latest" => {
                Some(json!({"hash": BLOCK_HASH, "number": BLOCK_NUMBER}))
            }
            [Value::String(selector), Value::Bool(false)] if selector == "finalized" => {
                Some(json!({"hash": FINALIZED_HASH, "number": FINALIZED_NUMBER}))
            }
            [Value::String(selector), Value::Bool(false)] if selector == BLOCK_NUMBER => {
                Some(json!({"hash": BLOCK_HASH, "number": BLOCK_NUMBER}))
            }
            _ => None,
        },
        "eth_getBalance"
            if params
                == [
                    Value::String(format!("{:#x}", state.sender)),
                    json!({"blockHash": BLOCK_HASH, "requireCanonical": true}),
                ] =>
        {
            Some(Value::String("0x64".to_owned()))
        }
        _ => None,
    };
    Json(match result {
        Some(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        None => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32602, "message": "invalid params"}
        }),
    })
}

fn exact_requested_hash(state: &LoopbackRpcState, params: &[Value]) -> Option<String> {
    let [Value::String(requested)] = params else {
        return None;
    };
    let retained = state.transaction_hash.lock().ok()?.clone()?;
    (requested == &retained).then_some(retained)
}

struct HistoryAudit {
    authorizations: BTreeMap<mfm_ids::AccessAttemptId, ContentRef>,
    observations: BTreeMap<mfm_ids::AccessAttemptId, ObservationOutcome>,
    capability_refs: BTreeSet<ContentRef>,
    root_outcome_ref: Option<ContentRef>,
    closed: bool,
}

impl HistoryAudit {
    fn from_batches(batches: &[CommittedBatch]) -> Self {
        let mut authorizations = BTreeMap::new();
        let mut observations = BTreeMap::new();
        let mut capability_refs = BTreeSet::new();
        let mut root_outcome_ref = None;
        for assigned in batches.iter().flat_map(|batch| &batch.records) {
            match &assigned.record {
                RunRecord::ExternalAccessAuthorized(authorization) => {
                    capability_refs.insert(authorization.capability_contract_ref.clone());
                    authorizations.insert(
                        authorization.access_attempt_id.clone(),
                        authorization.capability_contract_ref.clone(),
                    );
                }
                RunRecord::ExternalAccessObserved(observation) => {
                    observations.insert(
                        observation.access_attempt_id.clone(),
                        observation.outcome.clone(),
                    );
                }
                RunRecord::RunClosed(closed) => {
                    root_outcome_ref = Some(closed.outcome_ref.clone());
                }
                RunRecord::RunAdmitted(_) | RunRecord::StateTransitionCommitted(_) => {}
            }
        }
        let closed = root_outcome_ref.is_some();
        Self {
            authorizations,
            observations,
            capability_refs,
            root_outcome_ref,
            closed,
        }
    }

    fn root_outcome<T, F>(&self, batches: &[CommittedBatch]) -> Option<OperationOutcome<T, F>>
    where
        T: serde::de::DeserializeOwned,
        F: serde::de::DeserializeOwned,
    {
        let reference = self.root_outcome_ref.as_ref()?;
        let outcome = batches
            .iter()
            .flat_map(|batch| &batch.objects)
            .find(|object| &object.content_ref == reference)?
            .decode()
            .ok()?;
        match outcome {
            OperationOutcome::<LexicalValueRef, LexicalValueRef>::Success(reference) => {
                resolve_history_value(batches, &reference).map(OperationOutcome::Success)
            }
            OperationOutcome::<LexicalValueRef, LexicalValueRef>::Failure(reference) => {
                resolve_history_value(batches, &reference).map(OperationOutcome::Failure)
            }
        }
    }
}

fn resolve_history_value<T: serde::de::DeserializeOwned>(
    batches: &[CommittedBatch],
    reference: &LexicalValueRef,
) -> Option<T> {
    batches
        .iter()
        .flat_map(|batch| &batch.objects)
        .find(|object| object.content_ref == reference.value.value_ref)?
        .decode()
        .ok()
}

async fn broadcast_observation_exists(
    pool: &PgPool,
    run_id: &RunId,
    broadcast_capability_ref: &ContentRef,
) -> bool {
    capability_observation_exists(pool, run_id, broadcast_capability_ref).await
}

async fn capability_observation_exists(
    pool: &PgPool,
    run_id: &RunId,
    capability_ref: &ContentRef,
) -> bool {
    let batches = load_batches(pool, run_id).await;
    let audit = HistoryAudit::from_batches(&batches);
    audit.authorizations.iter().any(|(attempt, capability)| {
        capability == capability_ref
            && matches!(
                audit.observations.get(attempt),
                Some(ObservationOutcome::Returned { .. })
            )
    })
}

async fn run_worker(
    database: &TestDatabase,
    endpoint: &str,
    mode: &str,
    activation: &mfm_evm::WalletNonceDomainActivationAttestation,
    provider: &ProviderProcess,
) {
    let output = run_worker_process(database, endpoint, mode, activation, provider, None).await;
    assert_canaries_absent("worker stdout", &output.stdout);
    assert_canaries_absent("worker stderr", &output.stderr);
    assert!(
        output.status.success(),
        "EVM PostgreSQL worker {mode} failed with status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
}

async fn run_worker_expect_crash_after_broadcast(
    database: &TestDatabase,
    endpoint: &str,
    mode: &str,
    activation: &mfm_evm::WalletNonceDomainActivationAttestation,
    provider: &ProviderProcess,
) {
    let output = run_worker_process(
        database,
        endpoint,
        mode,
        activation,
        provider,
        Some(InjectedCrashBoundary::Broadcast),
    )
    .await;
    assert_injected_worker_crash(output, mode, "broadcast");
}

async fn run_worker_expect_crash_after_receipt(
    database: &TestDatabase,
    endpoint: &str,
    mode: &str,
    activation: &mfm_evm::WalletNonceDomainActivationAttestation,
    provider: &ProviderProcess,
) {
    let output = run_worker_process(
        database,
        endpoint,
        mode,
        activation,
        provider,
        Some(InjectedCrashBoundary::Receipt),
    )
    .await;
    assert_injected_worker_crash(output, mode, "receipt");
}

async fn run_worker_expect_crash_after_finality(
    database: &TestDatabase,
    endpoint: &str,
    mode: &str,
    activation: &mfm_evm::WalletNonceDomainActivationAttestation,
    provider: &ProviderProcess,
) {
    let output = run_worker_process(
        database,
        endpoint,
        mode,
        activation,
        provider,
        Some(InjectedCrashBoundary::Finality),
    )
    .await;
    assert_injected_worker_crash(output, mode, "finality");
}

async fn run_worker_expect_crash_before_completion_commit(
    database: &TestDatabase,
    endpoint: &str,
    mode: &str,
    activation: &mfm_evm::WalletNonceDomainActivationAttestation,
    provider: &mut ProviderProcess,
) {
    let commit_proxy = PostgresCommitFaultProxy::start(&database.database_url)
        .await
        .expect("start completion commit-fault proxy");
    let proxy_nonce_url = database_url_with_login(
        commit_proxy.database_url(),
        &database.nonce_application_url(),
    );
    let ready_directory = tempfile::tempdir().expect("create completion readiness directory");
    let ready_path = ready_directory.path().join("wallet-authority-ready");
    let intercept_target = commit_proxy
        .arm_after_statement(
            CommitFault::HoldTransactionBeforeCommit,
            1,
            "INSERT INTO wallet_nonce_completions",
        )
        .expect("arm completion pre-commit process-loss fault");
    let mut child = worker_command(
        database,
        endpoint,
        mode,
        activation,
        provider,
        None,
        WorkerCommandOptions {
            nonce_application_url: Some(&proxy_nonce_url),
            ready_path: Some(&ready_path),
            ..WorkerCommandOptions::default()
        },
    )
    .spawn()
    .expect("spawn completion pre-commit worker");
    if wait_for_worker_ready(&ready_path).await.is_err() {
        let _ = child.start_kill();
        let output = child
            .wait_with_output()
            .await
            .expect("wait for completion pre-commit worker after readiness timeout");
        commit_proxy.release_held_transactions();
        panic!(
            "completion pre-commit worker did not publish readiness (status {:?}): {}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    if tokio::time::timeout(
        COMPLETION_BOUNDARY_TIMEOUT,
        commit_proxy.wait_for_intercepts(intercept_target),
    )
    .await
    .is_err()
    {
        let _ = child.start_kill();
        let output = child
            .wait_with_output()
            .await
            .expect("wait for completion pre-commit worker after intercept timeout");
        commit_proxy.release_held_transactions();
        panic!(
            "completion pre-commit worker did not reach the commit boundary (status {:?}): {}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    child
        .start_kill()
        .expect("kill completion pre-commit worker");
    let output = child
        .wait_with_output()
        .await
        .expect("wait for killed completion pre-commit worker");
    commit_proxy.release_held_transactions();
    assert_canaries_absent("pre-completion-crash stdout", &output.stdout);
    assert_canaries_absent("pre-completion-crash stderr", &output.stderr);
    assert!(
        !output.status.success(),
        "completion pre-commit worker unexpectedly succeeded"
    );
    assert_eq!(
        output.status.code(),
        None,
        "completion pre-commit worker must be terminated by process loss"
    );
    let wallet_pool = database.wallet_pool().await;
    let retained = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wallet_nonce_completions")
        .fetch_one(&wallet_pool)
        .await
        .expect("count completion rows after pre-commit process loss");
    wallet_pool.close().await;
    assert_eq!(
        retained, 0,
        "pre-completion-commit process loss must leave no wallet completion"
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if provider
                .active_leases()
                .expect("read provider leases after pre-commit process loss")
                == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("drain provider lease after pre-commit process loss");
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
}

async fn run_worker_expect_completion_acknowledgement_loss(
    database: &TestDatabase,
    endpoint: &str,
    mode: &str,
    activation: &mfm_evm::WalletNonceDomainActivationAttestation,
    provider: &ProviderProcess,
) {
    let commit_proxy = PostgresCommitFaultProxy::start(&database.database_url)
        .await
        .expect("start completion acknowledgement proxy");
    let proxy_nonce_url = database_url_with_login(
        commit_proxy.database_url(),
        &database.nonce_application_url(),
    );
    let ready_directory = tempfile::tempdir().expect("create completion readiness directory");
    let ready_path = ready_directory.path().join("wallet-authority-ready");
    let intercept_target = commit_proxy
        .arm_after_statement(
            CommitFault::CommitAndLoseAcknowledgement,
            1,
            "INSERT INTO wallet_nonce_completions",
        )
        .expect("arm initial completion acknowledgement fault");
    let mut child = worker_command(
        database,
        endpoint,
        mode,
        activation,
        provider,
        None,
        WorkerCommandOptions {
            nonce_application_url: Some(&proxy_nonce_url),
            ready_path: Some(&ready_path),
            skip_projection_verify: true,
        },
    )
    .spawn()
    .expect("spawn completion acknowledgement worker");
    if wait_for_worker_ready(&ready_path).await.is_err() {
        let _ = child.start_kill();
        let output = child
            .wait_with_output()
            .await
            .expect("wait for completion acknowledgement worker after readiness timeout");
        commit_proxy.release_held_transactions();
        panic!(
            "completion acknowledgement worker did not publish readiness (status {:?}): {}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    if tokio::time::timeout(
        COMPLETION_BOUNDARY_TIMEOUT,
        commit_proxy.wait_for_intercepts(intercept_target),
    )
    .await
    .is_err()
    {
        let _ = child.start_kill();
        let output = child
            .wait_with_output()
            .await
            .expect("wait for completion acknowledgement worker after boundary timeout");
        commit_proxy.release_held_transactions();
        panic!(
            "completion acknowledgement worker did not reach the commit boundary (status {:?}): {}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    let mut stdout_reader = child
        .stdout
        .take()
        .expect("completion acknowledgement worker stdout");
    let mut stderr_reader = child
        .stderr
        .take()
        .expect("completion acknowledgement worker stderr");
    let status = match tokio::time::timeout(COMPLETION_BOUNDARY_TIMEOUT, child.wait()).await {
        Ok(status) => status.expect("run completion acknowledgement worker"),
        Err(_) => {
            let _ = child.start_kill();
            let status = child
                .wait()
                .await
                .expect("wait for timed-out completion acknowledgement worker");
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            stdout_reader
                .read_to_end(&mut stdout)
                .await
                .expect("read timed-out completion acknowledgement stdout");
            stderr_reader
                .read_to_end(&mut stderr)
                .await
                .expect("read timed-out completion acknowledgement stderr");
            commit_proxy.release_held_transactions();
            panic!(
                "completion acknowledgement worker did not settle (status {:?}): {}{}",
                status,
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr),
            );
        }
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    stdout_reader
        .read_to_end(&mut stdout)
        .await
        .expect("read completion acknowledgement stdout");
    stderr_reader
        .read_to_end(&mut stderr)
        .await
        .expect("read completion acknowledgement stderr");
    let output = Output {
        status,
        stdout,
        stderr,
    };
    assert_canaries_absent("completion-ack stdout", &output.stdout);
    assert_canaries_absent("completion-ack stderr", &output.stderr);
    assert!(
        output.status.success(),
        "completion acknowledgement worker failed with status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let wallet_pool = database.wallet_pool().await;
    let retained = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wallet_nonce_completions")
        .fetch_one(&wallet_pool)
        .await
        .expect("count completion rows after initial acknowledgement loss");
    wallet_pool.close().await;
    assert_eq!(
        retained, 1,
        "initial completion acknowledgement loss must retain exactly one row"
    );
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
}

async fn run_worker_expect_crash_after_completion(
    database: &TestDatabase,
    endpoint: &str,
    mode: &str,
    activation: &mfm_evm::WalletNonceDomainActivationAttestation,
    provider: &ProviderProcess,
) {
    let output = run_worker_process(
        database,
        endpoint,
        mode,
        activation,
        provider,
        Some(InjectedCrashBoundary::Completion),
    )
    .await;
    assert_injected_worker_crash(output, mode, "completion");
}

fn assert_injected_worker_crash(output: Output, mode: &str, boundary: &str) {
    assert_canaries_absent("crashed worker stdout", &output.stdout);
    assert_canaries_absent("crashed worker stderr", &output.stderr);
    assert!(
        !output.status.success(),
        "injected EVM PostgreSQL worker {mode} {boundary} crash unexpectedly succeeded"
    );
    assert_eq!(
        output.status.code(),
        Some(137),
        "injected EVM PostgreSQL worker {mode} {boundary} did not exit at the crash boundary"
    );
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
}

#[derive(Clone, Copy)]
enum InjectedCrashBoundary {
    Broadcast,
    Receipt,
    Finality,
    Completion,
}

#[derive(Default)]
struct WorkerCommandOptions<'a> {
    nonce_application_url: Option<&'a str>,
    ready_path: Option<&'a Path>,
    skip_projection_verify: bool,
}

async fn run_worker_process(
    database: &TestDatabase,
    endpoint: &str,
    mode: &str,
    activation: &mfm_evm::WalletNonceDomainActivationAttestation,
    provider: &ProviderProcess,
    crash_boundary: Option<InjectedCrashBoundary>,
) -> Output {
    worker_command(
        database,
        endpoint,
        mode,
        activation,
        provider,
        crash_boundary,
        WorkerCommandOptions::default(),
    )
    .output()
    .await
    .expect("run EVM PostgreSQL worker")
}

fn worker_command(
    database: &TestDatabase,
    endpoint: &str,
    mode: &str,
    activation: &mfm_evm::WalletNonceDomainActivationAttestation,
    provider: &ProviderProcess,
    crash_boundary: Option<InjectedCrashBoundary>,
    options: WorkerCommandOptions<'_>,
) -> tokio::process::Command {
    let mut command =
        tokio::process::Command::new(std::env::current_exe().expect("test executable"));
    let nonce_url = options
        .nonce_application_url
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| database.nonce_application_url());
    command
        .arg("--exact")
        .arg("evm_postgres_submission_worker")
        .arg("--nocapture")
        .env("DATABASE_URL", &database.database_url)
        .env(HISTORY_SCHEMA_ENV, &database.history_schema)
        .env(WALLET_SCHEMA_ENV, &database.wallet_schema)
        .env(RPC_ENDPOINT_ENV, endpoint)
        .env(
            ACTIVATION_ENV,
            serde_json::to_string(activation).expect("encode activation attestation"),
        )
        .env(PROVIDER_ENDPOINT_ENV, provider.endpoint())
        .env(PROVIDER_PUBLIC_KEY_ENV, provider.public_key_hex())
        .env(
            "MFM_TEST_RUN_READER_URL",
            database
                .history_run_reader_login
                .database_url(&database.database_url),
        )
        .env(
            "MFM_TEST_RUN_WRITER_URL",
            database
                .history_run_writer_login
                .database_url(&database.database_url),
        )
        .env(
            "MFM_TEST_CONFIG_READER_URL",
            database
                .history_config_reader_login
                .database_url(&database.database_url),
        )
        .env(
            "MFM_TEST_CONFIG_WRITER_URL",
            database
                .history_config_writer_login
                .database_url(&database.database_url),
        )
        .env(NONCE_APPLICATION_DATABASE_URL_ENV, nonce_url)
        .env(WORKER_MODE_ENV, mode)
        .env("RUST_MIN_STACK", WORKER_STACK_BYTES.to_string());
    for variable in [
        INJECTED_CRASH_AFTER_BROADCAST_ENV,
        INJECTED_CRASH_AFTER_RECEIPT_ENV,
        INJECTED_CRASH_AFTER_FINALITY_ENV,
        INJECTED_CRASH_AFTER_COMPLETION_ENV,
        SKIP_PRODUCTION_PROJECTION_VERIFY_ENV,
    ] {
        command.env_remove(variable);
    }
    if options.skip_projection_verify {
        command.env(SKIP_PRODUCTION_PROJECTION_VERIFY_ENV, "1");
    }
    command.env_remove(WORKER_READY_FILE_ENV);
    if let Some(path) = options.ready_path {
        command.env(WORKER_READY_FILE_ENV, path);
    }
    if let Some(boundary) = crash_boundary {
        let variable = match boundary {
            InjectedCrashBoundary::Broadcast => INJECTED_CRASH_AFTER_BROADCAST_ENV,
            InjectedCrashBoundary::Receipt => INJECTED_CRASH_AFTER_RECEIPT_ENV,
            InjectedCrashBoundary::Finality => INJECTED_CRASH_AFTER_FINALITY_ENV,
            InjectedCrashBoundary::Completion => INJECTED_CRASH_AFTER_COMPLETION_ENV,
        };
        command.env(variable, "1");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

async fn wait_for_worker_ready(path: &Path) -> Result<(), ()> {
    tokio::time::timeout(WORKER_STARTUP_TIMEOUT, async {
        loop {
            if std::fs::read(path)
                .map(|contents| contents == b"wallet-authority-ready")
                .unwrap_or(false)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| ())
}

fn expected_access_capabilities() -> BTreeSet<ContentRef> {
    BTreeSet::from([
        read_capability_ref::<ReadWalletNonceStatusCapability>(),
        read_capability_ref::<EvmPendingNonceCapability>(),
        read_capability_ref::<AttestCandidateIdentityCapability>(),
        read_capability_ref::<EvmTransactionLookupCapability>(),
        read_capability_ref::<EvmReceiptLookupCapability>(),
        read_capability_ref::<EvmFinalizedHeadCapability>(),
        read_capability_ref::<EvmInclusionBlockCapability>(),
        effect_capability_ref::<ReserveWalletNonceCapability>(),
        effect_capability_ref::<ActivateWalletCandidateCapability>(),
        effect_capability_ref::<BroadcastExactCandidateCapability>(),
        effect_capability_ref::<CompleteWalletNonceCapability>(),
    ])
}

fn read_capability_ref<C: RuntimeReadCapability>() -> ContentRef {
    C::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .expect("read capability contract")
}

fn effect_capability_ref<C: RuntimeEffectCapability>() -> ContentRef {
    C::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .expect("effect capability contract")
}

fn broadcast_capability_ref() -> ContentRef {
    effect_capability_ref::<BroadcastExactCandidateCapability>()
}

fn operation_id() -> StableId {
    stable("mfm.evm.integration/structured-submit")
}

fn stable(value: &str) -> StableId {
    StableId::new(value).expect("stable identity")
}

fn admission_object(object_type: &str, schema_name: &str, discriminator: u8) -> HistoryObject {
    HistoryObject::new(
        stable(object_type),
        SchemaId::new(
            schema_name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(format!("mfm.structured-schema.v1:{schema_name}:1").as_bytes()),
        )
        .expect("admission schema"),
        format!("{{\"discriminator\":{discriminator}}}"),
    )
    .expect("admission object")
}

fn certificate(name: &str, discriminator: u8) -> HistoryObject {
    HistoryObject::new(
        stable(&format!("mfm.evm.integration/{name}")),
        SchemaId::new(
            "mfm.evm.integration-certificate",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.structured-schema.v1:mfm.evm.integration-certificate:1"),
        )
        .expect("certificate schema"),
        format!("{{\"discriminator\":{discriminator}}}"),
    )
    .expect("certificate")
}

fn authority_reference(name: &str, discriminator: u8) -> EvmWalletReference {
    EvmWalletReference::from_content_ref(certificate(name, discriminator).content_ref)
}

fn single_release_history(
    admitted_routing_policy_ref: &ContentRef,
    physical_target_ref: ContentRef,
    certificate: HistoryObject,
) -> EvmPhysicalBindingReleaseHistory {
    EvmPhysicalBindingReleaseHistory::single(
        admitted_routing_policy_ref.clone(),
        physical_target_ref,
        certificate,
    )
    .expect("single physical release history")
}

struct TestDatabase {
    admin_pool: PgPool,
    database_url: String,
    history_schema: String,
    wallet_schema: String,
    history_run_reader_login: HistoryTestLogin,
    history_run_writer_login: HistoryTestLogin,
    history_config_reader_login: HistoryTestLogin,
    history_config_writer_login: HistoryTestLogin,
}

struct HistoryTestLogin {
    role_name: String,
    password: String,
}

impl TestDatabase {
    async fn create() -> Self {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL is required for parity tests");
        let admin_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("connect PostgreSQL administrator");
        let history_schema = unique_schema("history");
        let wallet_schema = unique_schema("wallet");
        for schema in [&history_schema, &wallet_schema] {
            sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
                .execute(&admin_pool)
                .await
                .expect("create isolated schema");
        }
        migrate_test_database(&scoped_database_url(&database_url, &history_schema))
            .await
            .expect("migrate structured history");
        PostgresEvmWalletSchema::migrate(&scoped_database_url(&database_url, &wallet_schema))
            .await
            .expect("migrate wallet authority");
        let mut wallet_login_admin = admin_pool
            .acquire()
            .await
            .expect("acquire wallet login administrator");
        configure_wallet_login_principals(&mut wallet_login_admin).await;
        drop(wallet_login_admin);
        let roles = load_history_target_roles(&database_url, &history_schema).await;
        let history_run_reader_login = HistoryTestLogin::create(
            &admin_pool,
            &history_schema,
            "rrd",
            &[&roles.qualification, &roles.run_reader],
        )
        .await;
        let history_run_writer_login = HistoryTestLogin::create(
            &admin_pool,
            &history_schema,
            "rwr",
            &[&roles.qualification, &roles.run_writer],
        )
        .await;
        let history_config_reader_login = HistoryTestLogin::create(
            &admin_pool,
            &history_schema,
            "crd",
            &[&roles.qualification, &roles.configuration_reader],
        )
        .await;
        let history_config_writer_login = HistoryTestLogin::create(
            &admin_pool,
            &history_schema,
            "cwr",
            &[&roles.qualification, &roles.configuration_writer],
        )
        .await;
        Self {
            admin_pool,
            database_url,
            history_schema,
            wallet_schema,
            history_run_reader_login,
            history_run_writer_login,
            history_config_reader_login,
            history_config_writer_login,
        }
    }

    fn activation_admin_url(&self) -> String {
        scoped_database_url(
            &required_database_url(ACTIVATION_ADMIN_DATABASE_URL_ENV),
            &self.wallet_schema,
        )
    }

    fn nonce_application_url(&self) -> String {
        scoped_database_url(
            &required_database_url(NONCE_APPLICATION_DATABASE_URL_ENV),
            &self.wallet_schema,
        )
    }

    async fn wallet_pool(&self) -> PgPool {
        isolated_pool(&self.database_url, &self.wallet_schema).await
    }

    async fn history_batches(&self, run_id: RunId) -> Vec<CommittedBatch> {
        let pool = isolated_pool(&self.database_url, &self.history_schema).await;
        let batches = load_batches(&pool, &run_id).await;
        pool.close().await;
        batches
    }

    async fn assert_canaries_absent_from_persistence(&self) {
        for schema in [&self.history_schema, &self.wallet_schema] {
            let tables = sqlx::query_scalar::<_, String>(
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = $1 AND table_type = 'BASE TABLE' ORDER BY table_name",
            )
            .bind(schema)
            .fetch_all(&self.admin_pool)
            .await
            .expect("inventory persisted canary surfaces");
            for table in tables {
                let qualified = format!(
                    "{}.{}",
                    quoted_test_identifier(schema),
                    quoted_test_identifier(&table)
                );
                for canary in [SECRET_CREDENTIAL_CANARY, PROVIDER_TEXT_CANARY] {
                    let retained = sqlx::query_scalar::<_, bool>(AssertSqlSafe(format!(
                        "SELECT EXISTS (SELECT 1 FROM {qualified} AS retained \
                         WHERE to_jsonb(retained)::text LIKE $1)"
                    )))
                    .bind(format!("%{canary}%"))
                    .fetch_one(&self.admin_pool)
                    .await
                    .expect("scan persisted canary surface");
                    assert!(!retained, "persisted table retained secret-bearing input");
                }
            }
        }
    }

    async fn cleanup(self) {
        for schema in [&self.history_schema, &self.wallet_schema] {
            sqlx::query(AssertSqlSafe(format!(
                "DROP SCHEMA IF EXISTS {schema} CASCADE"
            )))
            .execute(&self.admin_pool)
            .await
            .expect("drop isolated schema");
        }
        self.history_run_reader_login.drop(&self.admin_pool).await;
        self.history_run_writer_login.drop(&self.admin_pool).await;
        self.history_config_reader_login
            .drop(&self.admin_pool)
            .await;
        self.history_config_writer_login
            .drop(&self.admin_pool)
            .await;
        self.admin_pool.close().await;
    }
}

impl HistoryTestLogin {
    async fn create(
        admin_pool: &PgPool,
        schema: &str,
        purpose: &str,
        memberships: &[&str],
    ) -> Self {
        let digest = sha256_digest_bytes(schema.as_bytes()).to_string();
        let discriminator = &digest[..16];
        let role_name = history_login_role_name(schema, purpose);
        let password = format!("MfmEvmHistory{discriminator}{purpose}");
        sqlx::query(AssertSqlSafe(format!(
            "CREATE ROLE {role_name} LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE \
             NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD '{password}'"
        )))
        .execute(admin_pool)
        .await
        .expect("create restricted history login");
        let quoted = memberships
            .iter()
            .map(|name| format!("\"{}\"", name))
            .collect::<Vec<_>>()
            .join(", ");
        sqlx::query(AssertSqlSafe(format!(
            "GRANT {quoted} TO {role_name} WITH INHERIT FALSE, SET TRUE"
        )))
        .execute(admin_pool)
        .await
        .expect("grant exact history memberships");
        Self {
            role_name,
            password,
        }
    }

    fn database_url(&self, database_url: &str) -> String {
        database_url
            .parse::<PgConnectOptions>()
            .expect("parse history database URL")
            .username(&self.role_name)
            .password(&self.password)
            .to_url_lossy()
            .to_string()
    }

    async fn drop(self, admin_pool: &PgPool) {
        sqlx::query(AssertSqlSafe(format!("DROP ROLE {}", self.role_name)))
            .execute(admin_pool)
            .await
            .expect("drop restricted history login");
    }
}

fn history_login_role_name(schema: &str, purpose: &str) -> String {
    let digest = sha256_digest_bytes(schema.as_bytes()).to_string();
    let discriminator = &digest[..16];
    format!("mfm_evm_history_{purpose}_{discriminator}")
}

#[test]
fn history_login_purposes_produce_distinct_role_names() {
    let schema = "mfm_evm_submission_history_fixture";
    let run_reader = history_login_role_name(schema, "rrd");
    let run_writer = history_login_role_name(schema, "rwr");
    let configuration_reader = history_login_role_name(schema, "crd");
    let configuration_writer = history_login_role_name(schema, "cwr");

    assert_ne!(run_reader, run_writer);
    assert_ne!(configuration_reader, configuration_writer);
    assert!(run_reader.contains("_rrd_"));
    assert!(run_writer.contains("_rwr_"));
    assert!(configuration_reader.contains("_crd_"));
    assert!(configuration_writer.contains("_cwr_"));
}

fn quoted_test_identifier(value: &str) -> String {
    assert!(!value.is_empty());
    assert!(value.chars().enumerate().all(|(index, character)| {
        character == '_'
            || (character.is_ascii_alphanumeric() && (index > 0 || !character.is_ascii_digit()))
    }));
    format!("\"{value}\"")
}

fn required_database_url(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required for parity tests"))
}

fn database_url_with_active_role(database_url: &str, role: &str) -> String {
    let separator = if database_url.contains('?') { '&' } else { '?' };
    format!("{database_url}{separator}options%5Brole%5D={role}")
}

async fn configure_wallet_login_principals(admin: &mut PgConnection) {
    let role_urls = [
        (
            ACTIVATION_ADMIN_DATABASE_URL_ENV,
            "mfm_evm_wallet_activation_admin",
        ),
        (
            ACTIVATION_PUBLIC_DATABASE_URL_ENV,
            "mfm_evm_wallet_activation_public",
        ),
        (
            NONCE_APPLICATION_DATABASE_URL_ENV,
            "mfm_evm_wallet_nonce_application",
        ),
    ];
    let principals = role_urls.map(|(environment, role)| {
        let options = PgConnectOptions::from_str(&required_database_url(environment))
            .expect("parse wallet login database URL");
        (options.get_username().to_owned(), role)
    });
    assert!(principals[0].0 != principals[1].0);
    assert!(principals[0].0 != principals[2].0);
    assert!(principals[1].0 != principals[2].0);

    for (principal_name, granted_role) in principals {
        let principal = quoted_test_identifier(&principal_name);
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = $1)",
        )
        .bind(&principal_name)
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
        .expect("qualify wallet login principal");
        let inherited_roles = sqlx::query_scalar::<_, String>(
            "SELECT granted.rolname FROM pg_catalog.pg_auth_members AS membership \
             JOIN pg_catalog.pg_roles AS granted ON granted.oid = membership.roleid \
             JOIN pg_catalog.pg_roles AS member ON member.oid = membership.member \
             WHERE member.rolname = $1 ORDER BY granted.rolname",
        )
        .bind(&principal_name)
        .fetch_all(&mut *admin)
        .await
        .expect("inventory wallet login memberships");
        for inherited_role in inherited_roles {
            sqlx::query(AssertSqlSafe(format!(
                "REVOKE {} FROM {principal}",
                quoted_test_identifier(&inherited_role)
            )))
            .execute(&mut *admin)
            .await
            .expect("remove unrelated wallet login membership");
        }
        sqlx::query(AssertSqlSafe(format!(
            "GRANT {} TO {principal} WITH ADMIN FALSE, INHERIT FALSE, SET TRUE",
            quoted_test_identifier(granted_role)
        )))
        .execute(&mut *admin)
        .await
        .expect("grant exact wallet runtime role");
    }
}

async fn load_batches(pool: &PgPool, run_id: &RunId) -> Vec<CommittedBatch> {
    let rows = sqlx::query(
        "SELECT run_sequence::text AS run_sequence, batch_envelope_json \
           FROM run_history_batches WHERE run_id = $1 \
          ORDER BY run_history_batches.run_sequence",
    )
    .bind(run_id.as_str())
    .fetch_all(pool)
    .await
    .expect("load normalized structured history");
    let mut batches = Vec::with_capacity(rows.len());
    for row in rows {
        let run_sequence = row
            .try_get::<String, _>("run_sequence")
            .expect("batch sequence");
        let envelope_json = row
            .try_get::<String, _>("batch_envelope_json")
            .expect("batch envelope");
        assert!(envelope_json.len() <= MAX_RETAINED_DOCUMENT_BYTES);
        let mut envelope =
            serde_json::from_str::<Value>(&envelope_json).expect("strictly decode batch envelope");
        assert_eq!(
            canonical_json(&envelope)
                .expect("canonical batch envelope")
                .as_str(),
            envelope_json
        );
        let declared_object_count = envelope
            .get("object_count")
            .and_then(Value::as_u64)
            .expect("declared object count");
        let object_rows = sqlx::query(
            "SELECT object_type, content_schema_id, content_digest, canonical_json \
               FROM run_history_batch_objects \
              WHERE run_id = $1 AND run_sequence = $2::numeric ORDER BY object_ordinal",
        )
        .bind(run_id.as_str())
        .bind(run_sequence)
        .fetch_all(pool)
        .await
        .expect("load normalized batch objects");
        assert_eq!(
            usize::try_from(declared_object_count).expect("bounded declared object count"),
            object_rows.len()
        );
        let objects = object_rows
            .iter()
            .map(|object| {
                let canonical_object = object
                    .try_get::<String, _>("canonical_json")
                    .expect("canonical object bytes");
                assert!(canonical_object.len() <= MAX_RETAINED_DOCUMENT_BYTES);
                json!({
                    "object_type": object
                        .try_get::<String, _>("object_type")
                        .expect("object type"),
                    "content_ref": {
                        "schema_id": object
                            .try_get::<String, _>("content_schema_id")
                            .expect("content schema"),
                        "content_digest": object
                            .try_get::<String, _>("content_digest")
                            .expect("content digest"),
                    },
                    "canonical_json": canonical_object,
                })
            })
            .collect::<Vec<_>>();
        {
            let fields = envelope.as_object_mut().expect("batch envelope object");
            fields
                .remove("object_count")
                .expect("batch envelope object count");
            fields.insert("objects".to_owned(), Value::Array(objects));
        }
        let batch: CommittedBatch =
            serde_json::from_value(envelope).expect("reconstruct committed batch");
        assert!(batch.objects.iter().all(|object| object.validate().is_ok()));
        batches.push(batch);
    }
    batches
}

async fn isolated_pool(database_url: &str, schema: &str) -> PgPool {
    let options = PgConnectOptions::from_str(database_url)
        .expect("parse DATABASE_URL")
        .options([("search_path", schema)]);
    PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("connect isolated schema")
}

fn scoped_database_url(base_url: &str, schema: &str) -> String {
    let separator = if base_url.contains('?') { '&' } else { '?' };
    format!("{base_url}{separator}options=-csearch_path%3D{schema}")
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

fn unique_schema(kind: &str) -> String {
    let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    format!(
        "mfm_evm_submission_{kind}_{}_{}_{}",
        std::process::id(),
        timestamp,
        counter
    )
}

struct HistoryTargetRoles {
    qualification: String,
    run_reader: String,
    run_writer: String,
    configuration_reader: String,
    configuration_writer: String,
}

async fn load_history_target_roles(database_url: &str, schema: &str) -> HistoryTargetRoles {
    let pool = isolated_pool(database_url, schema).await;
    let row = sqlx::query(
        "SELECT qualification_role, run_reader_role, run_writer_role,                 configuration_reader_role, configuration_writer_role            FROM target_authority WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .expect("load history target roles");
    pool.close().await;
    HistoryTargetRoles {
        qualification: row.try_get("qualification_role").expect("qualification"),
        run_reader: row.try_get("run_reader_role").expect("run reader"),
        run_writer: row.try_get("run_writer_role").expect("run writer"),
        configuration_reader: row
            .try_get("configuration_reader_role")
            .expect("configuration reader"),
        configuration_writer: row
            .try_get("configuration_writer_role")
            .expect("configuration writer"),
    }
}
