use super::*;
use std::ops::Deref;

#[path = "services_read.rs"]
mod services_read;
#[path = "services_run.rs"]
mod services_run;

pub use self::services_read::RunReadServices;
pub(super) use self::services_read::{TrustedRunReader, VerifiedRunReadContext};

/// Application facade for certified typed runtime dispatch.
#[derive(Clone)]
pub struct RunServices<S, A> {
    pub(super) scheduler: SerialTypedScheduler,
    read: RunReadServices<S, A>,
    pub(super) execution_claim_heartbeat_interval: Duration,
}

impl<S, A> Deref for RunServices<S, A> {
    type Target = RunReadServices<S, A>;

    fn deref(&self) -> &Self::Target {
        &self.read
    }
}

impl<S, A> RunServices<S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    /// Creates typed async app services with an explicit trusted certification registry.
    pub fn new_with_certification_registry(
        scheduler: SerialTypedScheduler,
        store: S,
        artifacts: A,
        certification_registry: CertificationRegistry,
    ) -> Self {
        Self {
            scheduler,
            read: RunReadServices::new_with_certification_registry(
                store,
                artifacts,
                certification_registry,
            ),
            execution_claim_heartbeat_interval: default_execution_claim_heartbeat_interval(),
        }
    }

    async fn run_response_from_verified_status(
        &self,
        run_id: &RunId,
        status: DriveStatus,
    ) -> Result<RunResponse, PublicError> {
        let context = self
            .trusted_run_reader()
            .load_status_context(run_id)
            .await?;
        run_response_from_projection(
            run_id,
            context.runtime_spec(),
            context.events(),
            context.projection(),
            status,
        )
    }

    fn trusted_run_reader(&self) -> TrustedRunReader<'_, S, A> {
        TrustedRunReader::new(
            self.read.store(),
            self.read.artifacts(),
            self.read.certification_registry(),
        )
    }
}
