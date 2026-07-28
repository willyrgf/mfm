use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use mfm_evm::EvmWalletReference;
use mfm_executor::{
    ExecutorContractDescriptor, ExecutorDeployment, ResourceOwnership, ResourcePolicyBinding,
};
use mfm_ids::{RunId, StoreScopeId, TenantScopeId};
use mfm_signing::{
    SigningGenerationGuard, VerifiedGenerationGuardedSignerBinding,
    SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use sqlx::PgPool;

use crate::{
    AccessAuditPage, AccessTarget, AdmitRunRequest, AdmitRunResponse, DriveResponse,
    EntryPointContract, ErrorClass, ExportRequest, ExportedRunBytes, PageRequest, PublicError,
    PublicRunView, ReplayRequest, ReplayResponse, RunAccessGrant, RunAccessPolicy,
    SecretCredential, TransitionTracePage,
};

/// Deployment-owned inputs for the qualified EVM wallet executor.
///
/// This value contains no key material, unlock secret, RPC credential, or
/// signed transaction. Runtime paths remain in the separately parsed runtime
/// configuration, and the injected signing guard is checked on every transient
/// signing call.
pub struct EvmWalletDeployment {
    executor_pool: PgPool,
    executor_contract: ExecutorContractDescriptor,
    executor_deployment: ExecutorDeployment,
    resource_ownership: ResourceOwnership,
    resource_policy_binding: ResourcePolicyBinding,
    wallet_signer_binding_ref: EvmWalletReference,
    signer_binding: VerifiedGenerationGuardedSignerBinding,
    signer_generation_guard: Arc<dyn SigningGenerationGuard>,
}

impl EvmWalletDeployment {
    /// Constructs one exact wallet deployment before executable qualification.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        executor_pool: PgPool,
        executor_contract: ExecutorContractDescriptor,
        executor_deployment: ExecutorDeployment,
        resource_ownership: ResourceOwnership,
        resource_policy_binding: ResourcePolicyBinding,
        wallet_signer_binding_ref: EvmWalletReference,
        signer_binding: VerifiedGenerationGuardedSignerBinding,
        signer_generation_guard: Arc<dyn SigningGenerationGuard>,
    ) -> Result<Self, PublicError> {
        let ownership_ref = resource_ownership
            .reference()
            .map_err(|_| wallet_deployment_invalid())?;
        if executor_deployment.resource_ownership_ref() != Some(&ownership_ref)
            || executor_deployment.durable_ledger_generation_ref()
                != resource_ownership.durable_ledger_generation_ref()
            || signer_binding.durable_generation_ref()
                != executor_deployment.durable_ledger_generation_ref()
            || resource_ownership.destination_fencing_authority_ref()
                != Some(signer_binding.fence_attestation_ref())
            || signer_binding.provider_implementation_id().as_str()
                != mfm_keystore::KEYSTORE_SIGNING_IMPLEMENTATION_ID
            || signer_binding.algorithm().as_str() != SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID
            || signer_binding.profile().as_str() != SECP256K1_RFC6979_LOW_S_PROFILE_ID
            || wallet_signer_binding_ref.to_content_ref().is_err()
        {
            return Err(wallet_deployment_invalid());
        }
        Ok(Self {
            executor_pool,
            executor_contract,
            executor_deployment,
            resource_ownership,
            resource_policy_binding,
            wallet_signer_binding_ref,
            signer_binding,
            signer_generation_guard,
        })
    }

    pub(crate) fn into_parts(self) -> EvmWalletDeploymentParts {
        EvmWalletDeploymentParts {
            executor_pool: self.executor_pool,
            executor_contract: self.executor_contract,
            executor_deployment: self.executor_deployment,
            resource_ownership: self.resource_ownership,
            resource_policy_binding: self.resource_policy_binding,
            wallet_signer_binding_ref: self.wallet_signer_binding_ref,
            signer_binding: self.signer_binding,
            signer_generation_guard: self.signer_generation_guard,
        }
    }
}

pub(crate) struct EvmWalletDeploymentParts {
    pub(crate) executor_pool: PgPool,
    pub(crate) executor_contract: ExecutorContractDescriptor,
    pub(crate) executor_deployment: ExecutorDeployment,
    pub(crate) resource_ownership: ResourceOwnership,
    pub(crate) resource_policy_binding: ResourcePolicyBinding,
    pub(crate) wallet_signer_binding_ref: EvmWalletReference,
    pub(crate) signer_binding: VerifiedGenerationGuardedSignerBinding,
    pub(crate) signer_generation_guard: Arc<dyn SigningGenerationGuard>,
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

    /// Checks the authoritative store with one bounded writable-lineage probe.
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
        let target = AccessTarget::AdmitTarget {
            store_scope_id: self.store_scope_id.clone(),
            entry_point_operation_id: entry_point.entry_point_operation_id().clone(),
            invocation_identity: request.invocation_identity().clone(),
        };
        let tenant = self
            .policy
            .authorize(&credential, RunAccessGrant::Admit, &target)
            .await?
            .into_tenant_scope_id();
        self.backend.admit_run(tenant, entry_point, request).await
    }

    /// Authenticates, authorizes, and executes at most one legal run action.
    pub async fn drive_once(
        &self,
        credential: SecretCredential,
        run_id: RunId,
    ) -> Result<DriveResponse, PublicError> {
        let call = self
            .authorize_run(credential, RunAccessGrant::Drive, run_id)
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
            .authorize_run(credential, RunAccessGrant::ReadPublic, run_id)
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
            .authorize_run(credential, RunAccessGrant::Replay, run_id)
            .await?;
        if request.portable_export().is_some() {
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
            .authorize_run(credential, RunAccessGrant::InspectTrace, run_id)
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
            .authorize_run(credential, RunAccessGrant::InspectAudit, run_id)
            .await?;
        self.backend.read_access_audit(&call, page).await
    }

    /// Authenticates every dependency and returns final canonical portable-export bytes.
    pub async fn export_run(
        &self,
        credential: SecretCredential,
        run_id: RunId,
        request: ExportRequest,
    ) -> Result<ExportedRunBytes, PublicError> {
        let call = self
            .authorize_run(credential, RunAccessGrant::Export, run_id)
            .await?;
        self.backend.export_run(&call, request).await
    }

    async fn authorize_run(
        &self,
        credential: SecretCredential,
        grant: RunAccessGrant,
        run_id: RunId,
    ) -> Result<AuthorizedRunCall<'_>, PublicError> {
        let target = AccessTarget::RunTarget {
            store_scope_id: self.store_scope_id.clone(),
            run_id: run_id.clone(),
        };
        let tenant_scope_id = self
            .policy
            .authorize(&credential, grant, &target)
            .await?
            .into_tenant_scope_id();
        Ok(AuthorizedRunCall {
            credential,
            grant,
            policy: self.policy.as_ref(),
            store_scope_id: &self.store_scope_id,
            tenant_scope_id,
            run_id,
        })
    }
}

fn entry_point_not_found() -> PublicError {
    PublicError::not_found(
        "EntryPointNotFound",
        "The exact entry-point id is not registered",
    )
}

pub(crate) struct AuthorizedRunCall<'policy> {
    credential: SecretCredential,
    grant: RunAccessGrant,
    policy: &'policy dyn RunAccessPolicy,
    store_scope_id: &'policy StoreScopeId,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
}

impl AuthorizedRunCall<'_> {
    pub(crate) const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    pub(crate) const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Reauthorizes one additional grant for the same exact root and tenant.
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
        if authorized.tenant_scope_id() != &self.tenant_scope_id {
            return Err(PublicError::grant_denied());
        }
        Ok(())
    }

    /// Reauthorizes one required export dependency without copying the caller credential.
    pub(crate) async fn authorize_required_dependency(
        &self,
        run_id: RunId,
    ) -> Result<TenantScopeId, PublicError> {
        if self.grant != RunAccessGrant::Export {
            return Err(PublicError::grant_denied());
        }
        let target = AccessTarget::RunTarget {
            store_scope_id: self.store_scope_id.clone(),
            run_id,
        };
        let authorized = self
            .policy
            .authorize(&self.credential, self.grant, &target)
            .await
            .map_err(|error| match error {
                crate::AccessPolicyError::AuthenticationRequired => {
                    PublicError::authentication_required()
                }
                crate::AccessPolicyError::GrantDenied => PublicError::source_run_export_denied(),
            })?;
        if authorized.tenant_scope_id() != &self.tenant_scope_id {
            return Err(PublicError::source_run_export_denied());
        }
        Ok(authorized.into_tenant_scope_id())
    }

    /// Reauthorizes one trace source, preserving denial as a redacted source.
    pub(crate) async fn authorize_trace_source(
        &self,
        run_id: RunId,
    ) -> Result<Option<TenantScopeId>, PublicError> {
        if self.grant != RunAccessGrant::InspectTrace {
            return Err(PublicError::grant_denied());
        }
        let target = AccessTarget::RunTarget {
            store_scope_id: self.store_scope_id.clone(),
            run_id,
        };
        match self
            .policy
            .authorize(&self.credential, self.grant, &target)
            .await
        {
            Ok(authorized) if authorized.tenant_scope_id() == &self.tenant_scope_id => {
                Ok(Some(authorized.into_tenant_scope_id()))
            }
            Ok(_) | Err(crate::AccessPolicyError::GrantDenied) => Ok(None),
            Err(crate::AccessPolicyError::AuthenticationRequired) => {
                Err(PublicError::authentication_required())
            }
        }
    }
}

#[async_trait]
pub(crate) trait ApplicationBackend: Send + Sync {
    async fn check_ready(&self) -> Result<(), PublicError>;

    async fn admit_run(
        &self,
        tenant_scope_id: TenantScopeId,
        entry_point: EntryPointContract,
        request: AdmitRunRequest,
    ) -> Result<AdmitRunResponse, PublicError>;

    async fn drive_once(&self, call: &AuthorizedRunCall<'_>) -> Result<DriveResponse, PublicError>;

    async fn read_public_run(
        &self,
        call: &AuthorizedRunCall<'_>,
    ) -> Result<PublicRunView, PublicError>;

    async fn replay_run(
        &self,
        call: &AuthorizedRunCall<'_>,
        request: ReplayRequest,
    ) -> Result<ReplayResponse, PublicError>;

    async fn read_transition_trace(
        &self,
        call: &AuthorizedRunCall<'_>,
        page: PageRequest,
    ) -> Result<TransitionTracePage, PublicError>;

    async fn read_access_audit(
        &self,
        call: &AuthorizedRunCall<'_>,
        page: PageRequest,
    ) -> Result<AccessAuditPage, PublicError>;

    async fn export_run(
        &self,
        call: &AuthorizedRunCall<'_>,
        request: ExportRequest,
    ) -> Result<ExportedRunBytes, PublicError>;
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
        TestApplicationBackend { mode },
    )
}

#[cfg(any(test, feature = "test-support"))]
struct TestApplicationBackend {
    mode: TestApplicationMode,
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
        _tenant_scope_id: TenantScopeId,
        _entry_point: EntryPointContract,
        _request: AdmitRunRequest,
    ) -> Result<AdmitRunResponse, PublicError> {
        Err(self.failure())
    }

    async fn drive_once(
        &self,
        _call: &AuthorizedRunCall<'_>,
    ) -> Result<DriveResponse, PublicError> {
        Err(self.failure())
    }

    async fn read_public_run(
        &self,
        _call: &AuthorizedRunCall<'_>,
    ) -> Result<PublicRunView, PublicError> {
        Err(self.failure())
    }

    async fn replay_run(
        &self,
        _call: &AuthorizedRunCall<'_>,
        _request: ReplayRequest,
    ) -> Result<ReplayResponse, PublicError> {
        match &self.mode {
            TestApplicationMode::Replay(response) => Ok(response.clone()),
            TestApplicationMode::Sentinel | TestApplicationMode::RunNotFound => Err(self.failure()),
        }
    }

    async fn read_transition_trace(
        &self,
        _call: &AuthorizedRunCall<'_>,
        _page: PageRequest,
    ) -> Result<TransitionTracePage, PublicError> {
        Err(self.failure())
    }

    async fn read_access_audit(
        &self,
        _call: &AuthorizedRunCall<'_>,
        _page: PageRequest,
    ) -> Result<AccessAuditPage, PublicError> {
        Err(self.failure())
    }

    async fn export_run(
        &self,
        _call: &AuthorizedRunCall<'_>,
        _request: ExportRequest,
    ) -> Result<ExportedRunBytes, PublicError> {
        Err(self.failure())
    }
}

/// Connects a production application using independently fenced run and wallet authorities.
///
/// Production exact reproduction is deliberately unavailable in this cutover. `reproduce`
/// returns the frozen `unavailable` result and never falls back to live runtime capabilities.
pub async fn connect_production_application<RunFence, ExecutorFence>(
    database_url: Option<&str>,
    runtime_config_path: Option<&Path>,
    policy: Arc<dyn RunAccessPolicy>,
    deployment_writer_fence: RunFence,
    wallet: EvmWalletDeployment,
    executor_writer_fence: ExecutorFence,
) -> Result<Application, PublicError>
where
    RunFence: mfm_storage_postgres::AuthoritativeWriterFence + 'static,
    ExecutorFence: mfm_storage_executor_postgres::ExecutorWriterGenerationFence + 'static,
{
    crate::production::connect(
        database_url,
        runtime_config_path,
        policy,
        deployment_writer_fence,
        wallet,
        executor_writer_fence,
    )
    .await
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::AuthorizedRunCall;
    use crate::{
        AccessPolicyError, AccessTarget, AuthorizedTenant, RunAccessGrant, RunAccessPolicy,
        SecretCredential,
    };
    use async_trait::async_trait;
    use mfm_ids::{RunId, StoreScopeId, TenantScopeId};

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

    struct CountingPolicy {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl RunAccessPolicy for CountingPolicy {
        async fn authorize(
            &self,
            _credential: &SecretCredential,
            _grant: RunAccessGrant,
            _target: &AccessTarget,
        ) -> Result<AuthorizedTenant, AccessPolicyError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(AccessPolicyError::GrantDenied)
        }
    }

    #[tokio::test]
    async fn export_dependencies_have_their_own_redacted_denial_contract() {
        let root_tenant = tenant('1');
        let run_id = run_id();
        let store_scope_id = store_scope();

        let granted = FixedPolicy {
            expected_grant: RunAccessGrant::Export,
            result: Ok(AuthorizedTenant::new(root_tenant.clone())),
        };
        let call = authorized_call(
            &granted,
            &store_scope_id,
            root_tenant.clone(),
            run_id.clone(),
            RunAccessGrant::Export,
        );
        assert_eq!(
            call.authorize_required_dependency(run_id.clone())
                .await
                .expect("authorized source export"),
            root_tenant
        );

        let denied = FixedPolicy {
            expected_grant: RunAccessGrant::Export,
            result: Err(AccessPolicyError::GrantDenied),
        };
        let call = authorized_call(
            &denied,
            &store_scope_id,
            root_tenant.clone(),
            run_id.clone(),
            RunAccessGrant::Export,
        );
        let error = call
            .authorize_required_dependency(run_id.clone())
            .await
            .expect_err("denied source export");
        assert_eq!(error.code, "SourceRunExportDenied");

        let other_tenant = FixedPolicy {
            expected_grant: RunAccessGrant::Export,
            result: Ok(AuthorizedTenant::new(tenant('2'))),
        };
        let call = authorized_call(
            &other_tenant,
            &store_scope_id,
            root_tenant,
            run_id.clone(),
            RunAccessGrant::Export,
        );
        let error = call
            .authorize_required_dependency(run_id)
            .await
            .expect_err("cross-tenant source export");
        assert_eq!(error.code, "SourceRunExportDenied");
    }

    #[tokio::test]
    async fn non_verify_replay_requires_a_separate_same_tenant_export_grant() {
        let root_tenant = tenant('1');
        let run_id = run_id();
        let store_scope_id = store_scope();

        let granted = FixedPolicy {
            expected_grant: RunAccessGrant::Export,
            result: Ok(AuthorizedTenant::new(root_tenant.clone())),
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
            Ok(AuthorizedTenant::new(tenant('2'))),
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
            assert_eq!(error.code, "GrantDenied");
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
        assert_eq!(error.code, "AuthenticationRequired");
    }

    #[tokio::test]
    async fn trace_source_denial_is_redacted_but_reauthentication_failure_is_not() {
        let run_id = run_id();
        let store_scope_id = store_scope();
        let root_tenant = tenant('1');

        let granted = FixedPolicy {
            expected_grant: RunAccessGrant::InspectTrace,
            result: Ok(AuthorizedTenant::new(root_tenant.clone())),
        };
        let call = authorized_call(
            &granted,
            &store_scope_id,
            root_tenant.clone(),
            run_id.clone(),
            RunAccessGrant::InspectTrace,
        );
        assert_eq!(
            call.authorize_trace_source(run_id.clone())
                .await
                .expect("authorized trace decision"),
            Some(root_tenant.clone())
        );

        for result in [
            Err(AccessPolicyError::GrantDenied),
            Ok(AuthorizedTenant::new(tenant('2'))),
        ] {
            let policy = FixedPolicy {
                expected_grant: RunAccessGrant::InspectTrace,
                result,
            };
            let call = authorized_call(
                &policy,
                &store_scope_id,
                root_tenant.clone(),
                run_id.clone(),
                RunAccessGrant::InspectTrace,
            );
            assert_eq!(
                call.authorize_trace_source(run_id.clone())
                    .await
                    .expect("redacted trace decision"),
                None
            );
        }

        let unauthenticated = FixedPolicy {
            expected_grant: RunAccessGrant::InspectTrace,
            result: Err(AccessPolicyError::AuthenticationRequired),
        };
        let call = authorized_call(
            &unauthenticated,
            &store_scope_id,
            root_tenant,
            run_id.clone(),
            RunAccessGrant::InspectTrace,
        );
        let error = call
            .authorize_trace_source(run_id)
            .await
            .expect_err("reauthentication failure");
        assert_eq!(error.code, "AuthenticationRequired");
    }

    #[tokio::test]
    async fn dependency_helpers_reject_cross_purpose_use_before_policy_lookup() {
        let policy = CountingPolicy {
            calls: AtomicUsize::new(0),
        };
        let store_scope_id = store_scope();
        let tenant_scope_id = tenant('1');
        let run_id = run_id();

        let trace_call = authorized_call(
            &policy,
            &store_scope_id,
            tenant_scope_id.clone(),
            run_id.clone(),
            RunAccessGrant::InspectTrace,
        );
        let error = trace_call
            .authorize_required_dependency(run_id.clone())
            .await
            .expect_err("trace authority cannot authorize export dependencies");
        assert_eq!(error.code, "GrantDenied");

        let export_call = authorized_call(
            &policy,
            &store_scope_id,
            tenant_scope_id,
            run_id.clone(),
            RunAccessGrant::Export,
        );
        let error = export_call
            .authorize_trace_source(run_id)
            .await
            .expect_err("export authority cannot authorize trace sources");
        assert_eq!(error.code, "GrantDenied");
        assert_eq!(policy.calls.load(Ordering::SeqCst), 0);
    }

    fn authorized_call<'policy>(
        policy: &'policy dyn RunAccessPolicy,
        store_scope_id: &'policy StoreScopeId,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
        grant: RunAccessGrant,
    ) -> AuthorizedRunCall<'policy> {
        AuthorizedRunCall {
            credential: SecretCredential::new(b"opaque".to_vec()).expect("credential"),
            grant,
            policy,
            store_scope_id,
            tenant_scope_id,
            run_id,
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

    fn run_id() -> RunId {
        RunId::parse(
            "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("run id")
    }
}
