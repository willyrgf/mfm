#![warn(missing_docs)]
//! PostgreSQL representation of structured runtime history and configuration.
//!
//! Authority-bearing use starts through the opaque target-session openers. Schema
//! migration uses the separate owner path exposed by [`PostgresSchema`]. Ordinary
//! application code never receives a pool, URL, connection option, raw fence, or
//! DML transaction handle.

mod checkpoint;
mod configuration;
mod error;
mod qualification;
mod roles;
mod schema;
mod session;
#[cfg(test)]
mod sql_inventory;
mod structured;
mod transaction;

pub use checkpoint::{
    CheckpointError, CheckpointKey, CheckpointState, CheckpointStream, ExternalCheckpointAuthority,
    ExternalCheckpointLedger,
};
pub use configuration::PostgresConfigurationHistoryBackend;
pub use error::{PostgresStoreError, Result};
pub use qualification::{
    open_configuration_maintenance, open_structured_authoritative,
    open_structured_authoritative_application, open_structured_authoritative_with_configuration,
};
pub use schema::PostgresSchema;
#[cfg(feature = "test-support")]
pub use session::{
    issue_application_sessions, issue_combined_sessions, issue_configuration_maintenance_sessions,
    SessionLoginMaterial, TargetSessionMaterials,
};
pub use session::{
    open_application_sessions, open_combined_sessions, open_configuration_sessions,
    DeploymentCredentialBroker, PostgresApplicationSessions, PostgresCombinedSessions,
    PostgresConfigurationSessions, PostgresTargetAdmission, TargetBinding,
};
#[cfg(feature = "test-support")]
pub use session::{
    PostgresApplicationSessions as ApplicationTargetSessions,
    PostgresCombinedSessions as CombinedTargetSessions,
    PostgresConfigurationSessions as ConfigurationMaintenanceSessions,
};
pub use structured::PostgresStructuredHistoryBackend;

#[cfg(all(test, feature = "parity-tests"))]
mod tests {
    use sqlx::postgres::PgPoolOptions;

    use crate::schema::validate_authoritative_schema;

    #[tokio::test]
    #[ignore = "the managed SQLx task probes its caller-selected schema"]
    async fn verification_probe_accepts_the_current_authoritative_schema() {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL is required for the schema probe");
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("connect the caller-selected schema");
        validate_authoritative_schema(&pool)
            .await
            .expect("current schema must match the authoritative structured-history model");
        pool.close().await;
    }
}
