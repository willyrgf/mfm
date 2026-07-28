use super::*;
use std::ops::Deref;

#[path = "services_read.rs"]
mod services_read;
#[path = "services_run.rs"]
mod services_run;

pub use self::services_read::RunReadServices;
pub(super) use self::services_read::{
    TrustedRunReader, VerifiedRunReadContext, VerifiedStatusReadContext,
};

/// Application facade for certified typed runtime dispatch.
#[derive(Clone)]
pub struct RunServices<S> {
    pub(super) scheduler: SerialTypedScheduler,
    read: RunReadServices<S>,
    pub(super) execution_claim_heartbeat_interval: Duration,
}

impl<S> Deref for RunServices<S> {
    type Target = RunReadServices<S>;

    fn deref(&self) -> &Self::Target {
        &self.read
    }
}

impl<S> RunServices<S>
where
    S: store::RunJournalStore + store::StoreScopeStore + Send + Sync + 'static,
{
    /// Creates typed async app services with an explicit trusted certification registry.
    pub fn new_with_certification_registry(
        scheduler: SerialTypedScheduler,
        store: Arc<S>,
        certification_registry: CertificationRegistry,
    ) -> Self {
        Self {
            scheduler,
            read: RunReadServices::new_with_certification_registry(store, certification_registry),
            execution_claim_heartbeat_interval: default_execution_claim_heartbeat_interval(),
        }
    }

    async fn run_response_from_verified_current(
        &self,
        current: &VerifiedCurrentRun,
        status: DriveStatus,
    ) -> Result<RunResponse, PublicError>
    where
        S: store::CurrentProjectionStore,
    {
        let resource_lane_projection = self
            .read
            .store()
            .status_projection_snapshot(current.view().run_id())
            .await
            .map_err(async_app_store_error)?;
        run_response_from_verified_current(current, &resource_lane_projection, status.as_str())
    }

    fn trusted_run_reader(&self) -> TrustedRunReader<'_, S> {
        TrustedRunReader::new(self.read.store(), self.read.certification_registry())
    }
}
