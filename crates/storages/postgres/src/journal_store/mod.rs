mod append;
mod fact_scan;
mod load;
mod rows;
mod support;

#[cfg(all(test, feature = "parity-tests"))]
pub(crate) use append::run_advisory_lock_key;

use mfm_store::v2::{
    AdmissionSourceBackend, AdmissionSourceVerifier, AdmittedSupportGraph, AppendOutcome,
    AsyncStoreFuture, CommittedRunJournal, FactAttestationLoadVerifier, FactScanBackend,
    FactScanPage, FactScanPageVerifier, JournalAppendVerifier, JournalLoadVerifier,
    PersistedFactScanAttestation, RunHistoryReadinessBackend, RunJournalBackend,
    StoreAuthorityContext, SupportBackend, SupportGraphAdmissionVerifier, VerifiedAdmissionSources,
};
#[cfg(any(test, feature = "parity-tests"))]
use mfm_store::v2::{RunHistoryAdmissionLockTestBackend, RunHistoryCommitFailureBackend};

use crate::error::PostgresStoreError;
use crate::store::PostgresRunJournalBackend;

#[cfg(all(test, feature = "parity-tests"))]
pub(crate) async fn verify_persisted_run_for_test(
    connection: &mut sqlx::PgConnection,
    store_identity: &mfm_store::v2::StoreIdentity,
    tenant_scope_id: &mfm_ids::TenantScopeId,
    run_id: &mfm_ids::RunId,
) -> crate::Result<()> {
    rows::load_run(connection, store_identity, tenant_scope_id, run_id)
        .await
        .map(drop)
}

impl RunJournalBackend for PostgresRunJournalBackend {
    type Error = PostgresStoreError;

    fn store_authority_context(&self) -> &StoreAuthorityContext {
        self.store_authority_context()
    }

    fn backend_append<'a>(
        &'a self,
        verifier: JournalAppendVerifier,
    ) -> AsyncStoreFuture<'a, AppendOutcome, Self::Error> {
        Box::pin(async move { append::append(self, verifier).await })
    }

    fn backend_load<'a>(
        &'a self,
        verifier: JournalLoadVerifier,
    ) -> AsyncStoreFuture<'a, CommittedRunJournal, Self::Error> {
        Box::pin(async move { load::load(self, verifier).await })
    }
}

impl AdmissionSourceBackend for PostgresRunJournalBackend {
    fn backend_verify_admission_sources<'a>(
        &'a self,
        mut verifier: AdmissionSourceVerifier,
    ) -> AsyncStoreFuture<'a, VerifiedAdmissionSources, Self::Error> {
        Box::pin(async move {
            while let Some(load) = verifier.pending_source_load_verifier() {
                let journal = self.backend_load(load).await?;
                verifier.accept_verified_source(journal)?;
            }
            verifier.complete().map_err(Into::into)
        })
    }
}

impl FactScanBackend for PostgresRunJournalBackend {
    fn backend_fact_scan_page<'a>(
        &'a self,
        verifier: FactScanPageVerifier,
    ) -> AsyncStoreFuture<'a, FactScanPage, Self::Error> {
        Box::pin(async move { fact_scan::scan_page(self, verifier).await })
    }

    fn backend_load_fact_attestations<'a>(
        &'a self,
        verifier: FactAttestationLoadVerifier,
    ) -> AsyncStoreFuture<'a, Vec<PersistedFactScanAttestation>, Self::Error> {
        Box::pin(async move { fact_scan::load_attestations(self, verifier).await })
    }
}

impl SupportBackend for PostgresRunJournalBackend {
    fn backend_admit_support_graph<'a>(
        &'a self,
        verifier: SupportGraphAdmissionVerifier,
    ) -> AsyncStoreFuture<'a, AdmittedSupportGraph, Self::Error> {
        Box::pin(async move { support::admit_graph(self, verifier).await })
    }
}

impl RunHistoryReadinessBackend for PostgresRunJournalBackend {
    fn backend_check_ready(&self) -> AsyncStoreFuture<'_, (), Self::Error> {
        Box::pin(async move { self.check_ready().await })
    }
}

#[cfg(any(test, feature = "parity-tests"))]
impl RunHistoryCommitFailureBackend for PostgresRunJournalBackend {
    type FailurePoint = crate::store::TestCommitFailurePoint;

    fn backend_inject_commit_failure(
        &self,
        run_id: mfm_ids::RunId,
        batch_purpose: mfm_journal::v2::BatchPurpose,
        point: Self::FailurePoint,
    ) -> Result<(), Self::Error> {
        self.inject_commit_failure(run_id, batch_purpose, point)
    }

    fn backend_commit_failure_is_armed(&self) -> Result<bool, Self::Error> {
        self.commit_failure_is_armed()
    }

    fn backend_take_commit_failure(
        &self,
        run_id: &mfm_ids::RunId,
        batch_purpose: mfm_journal::v2::BatchPurpose,
    ) -> Result<Option<Self::FailurePoint>, Self::Error> {
        self.take_commit_failure(run_id, batch_purpose)
    }
}

#[cfg(any(test, feature = "parity-tests"))]
impl RunHistoryAdmissionLockTestBackend for PostgresRunJournalBackend {
    type Hook = crate::store::TestAdmissionRunLockHook;

    fn backend_inject_before_admission_run_lock(
        &self,
        append_request_id: mfm_ids::AppendRequestId,
    ) -> Result<Self::Hook, Self::Error> {
        self.inject_before_admission_run_lock(append_request_id)
    }
}
