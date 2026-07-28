mod append;
mod fact_scan;
mod load;
mod rows;
mod support;

#[cfg(all(test, feature = "parity-tests"))]
pub(crate) use append::run_advisory_lock_key;

use mfm_store::v1::{
    AdmissionSourceBackend, AdmissionSourceVerifier, AdmittedSupportGraph, AppendOutcome,
    AsyncStoreFuture, CommittedRunJournal, FactAttestationLoadVerifier, FactScanBackend,
    FactScanPage, FactScanPageVerifier, JournalAppendVerifier, JournalLoadVerifier,
    PersistedFactScanAttestation, RunJournalBackend, StoreAuthorityContext, SupportBackend,
    SupportGraphAdmissionVerifier, VerifiedAdmissionSources,
};

use crate::error::PostgresStoreError;
use crate::store::QualifiedPostgresStore;

#[cfg(all(test, feature = "parity-tests"))]
pub(crate) async fn verify_persisted_run_for_test(
    connection: &mut sqlx::PgConnection,
    store: &QualifiedPostgresStore,
    tenant_scope_id: &mfm_ids::TenantScopeId,
    run_id: &mfm_ids::RunId,
) -> crate::Result<()> {
    rows::load_run(
        connection,
        store.store_authority_context().store_identity(),
        tenant_scope_id,
        run_id,
    )
    .await
    .map(drop)
}

impl RunJournalBackend for QualifiedPostgresStore {
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

impl AdmissionSourceBackend for QualifiedPostgresStore {
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

impl FactScanBackend for QualifiedPostgresStore {
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

impl SupportBackend for QualifiedPostgresStore {
    fn backend_admit_support_graph<'a>(
        &'a self,
        verifier: SupportGraphAdmissionVerifier,
    ) -> AsyncStoreFuture<'a, AdmittedSupportGraph, Self::Error> {
        Box::pin(async move { support::admit_graph(self, verifier).await })
    }
}
