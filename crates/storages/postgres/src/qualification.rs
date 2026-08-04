//! Production openers that consume opaque exact-target session bundles only.

use std::sync::Arc;

use mfm_certify::structured::QualifiedProgramRegistry;
use mfm_store::structured::{
    assemble_structured_runtime, AssembledStructuredRuntime, ConfigurationHistoryStore,
    PublicPhysicalBindingVerifier,
};

use crate::configuration::PostgresConfigurationHistoryBackend;
use crate::error::Result;
use crate::session::{
    PostgresApplicationSessions, PostgresCombinedSessions, PostgresConfigurationSessions,
};
use crate::structured::PostgresStructuredHistoryBackend;

/// Opens the sole structured RunHistory store from an opaque application session bundle.
///
/// The bundle must already bind database identity, schema, store scope, role identities,
/// release epoch, and fence generation. Missing deployment authority fails closed; there is
/// no memory fallback and no raw pool, URL, or fence input.
pub async fn open_structured_authoritative(
    sessions: PostgresApplicationSessions,
    registry: QualifiedProgramRegistry,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
) -> Result<AssembledStructuredRuntime<PostgresStructuredHistoryBackend>> {
    let backend = PostgresStructuredHistoryBackend::from_application_sessions(sessions);
    Ok(assemble_structured_runtime(
        backend,
        registry,
        physical_binding_verifier,
    ))
}

/// Opens structured RunHistory together with append-only configured-value history.
///
/// Combined sessions include physically distinct run and configuration writer capabilities.
/// Assembly splits configuration once: deployment retention keeps the non-cloneable writer,
/// while application admission receives only resolve capability.
pub async fn open_structured_authoritative_with_configuration(
    sessions: PostgresCombinedSessions,
    registry: QualifiedProgramRegistry,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
) -> Result<(
    AssembledStructuredRuntime<PostgresStructuredHistoryBackend>,
    ConfigurationHistoryStore<PostgresConfigurationHistoryBackend>,
)> {
    let (run_reader, run_writer, configuration_reader, configuration_writer, target) =
        sessions.into_parts();
    let configuration_backend = PostgresConfigurationHistoryBackend::from_sessions(
        configuration_reader,
        Some(configuration_writer),
        target.clone(),
    );
    let run_backend =
        PostgresStructuredHistoryBackend::from_run_parts(run_reader, run_writer, target);
    let assembled = assemble_structured_runtime(run_backend, registry, physical_binding_verifier);
    Ok((
        assembled,
        ConfigurationHistoryStore::new(configuration_backend),
    ))
}

/// Opens the application boundary with structured RunHistory and resolve-only configuration.
pub async fn open_structured_authoritative_application(
    sessions: PostgresApplicationSessions,
    registry: QualifiedProgramRegistry,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
) -> Result<(
    AssembledStructuredRuntime<PostgresStructuredHistoryBackend>,
    mfm_store::structured::ConfigurationHistoryReader<PostgresConfigurationHistoryBackend>,
)> {
    let (run_reader, run_writer, configuration_reader, target) = sessions.into_application_parts();
    let configuration =
        ConfigurationHistoryStore::new(PostgresConfigurationHistoryBackend::from_sessions(
            configuration_reader,
            None,
            target.clone(),
        ));
    let assembled = assemble_structured_runtime(
        PostgresStructuredHistoryBackend::from_run_parts(run_reader, run_writer, target),
        registry,
        physical_binding_verifier,
    );
    let (_writer, reader) = configuration.split();
    Ok((assembled, reader))
}

/// Opens the separately held deployment-maintenance authority for append-only configuration.
pub async fn open_configuration_maintenance(
    sessions: PostgresConfigurationSessions,
) -> Result<mfm_store::structured::ConfigurationHistoryWriter<PostgresConfigurationHistoryBackend>>
{
    let (configuration_reader, configuration_writer, target) = sessions.into_parts();
    let configuration =
        ConfigurationHistoryStore::new(PostgresConfigurationHistoryBackend::from_sessions(
            configuration_reader,
            Some(configuration_writer),
            target,
        ));
    let (writer, _reader) = configuration.split();
    Ok(writer)
}
