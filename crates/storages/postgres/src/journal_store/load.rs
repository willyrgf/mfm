use mfm_store::{CommittedRunJournal, JournalLoadVerifier, StoreError};

use crate::error::{database_error, Result};
use crate::store::PostgresRunJournalBackend;

use super::rows::load_run;

pub(super) async fn load(
    store: &PostgresRunJournalBackend,
    verifier: JournalLoadVerifier,
) -> Result<CommittedRunJournal> {
    let mut transaction = store
        .writer_pool()
        .begin()
        .await
        .map_err(|error| database_error("begin journal load", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set journal load isolation", error))?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("set journal load read only", error))?;
    store
        .pin_transaction_schema(&mut transaction)
        .await
        .map_err(|error| database_error("pin qualified journal schema", error))?;
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .map_err(|error| database_error("assume journal application role", error))?;

    let loaded = load_run(
        &mut transaction,
        store.store_authority_context().store_identity(),
        verifier.tenant_scope_id(),
        verifier.run_id(),
    )
    .await?;
    let Some(loaded) = loaded else {
        return Err(StoreError::RunNotFound.into());
    };
    let journal = verifier.verify(loaded.commits, loaded.objects)?;
    transaction
        .commit()
        .await
        .map_err(|error| database_error("commit journal load snapshot", error))?;
    Ok(journal)
}
