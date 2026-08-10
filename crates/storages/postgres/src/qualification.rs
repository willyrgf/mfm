//! Production openers that consume opaque exact-target session bundles only.

use std::sync::Arc;

use mfm_certify::structured::CertifiedProgramRegistry;
use mfm_store::structured::{
    qualify_and_open_configuration_history, qualify_and_open_structured_store,
    ConfigurationHistoryStore, OpenedStructuredStore, PhysicalObligationChecker,
};

use crate::configuration::PostgresConfigurationHistoryBackend;
use crate::error::{PostgresStoreError, Result};
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
    registry: CertifiedProgramRegistry,
    physical_binding_verifier: Arc<dyn PhysicalObligationChecker>,
) -> Result<OpenedStructuredStore<PostgresStructuredHistoryBackend>> {
    let backend = PostgresStructuredHistoryBackend::from_application_sessions(sessions);
    qualify_and_open_structured_store(backend, registry, physical_binding_verifier)
        .await
        .map_err(|_| PostgresStoreError::Corruption("runtime semantic open failed"))
}

/// Opens structured RunHistory together with append-only configured-value history.
///
/// Combined sessions include physically distinct run and configuration writer capabilities.
/// Assembly splits configuration once: deployment retention keeps the non-cloneable writer,
/// while application admission receives only resolve capability.
pub async fn open_structured_authoritative_with_configuration(
    sessions: PostgresCombinedSessions,
    registry: CertifiedProgramRegistry,
    physical_binding_verifier: Arc<dyn PhysicalObligationChecker>,
) -> Result<(
    OpenedStructuredStore<PostgresStructuredHistoryBackend>,
    ConfigurationHistoryStore<PostgresConfigurationHistoryBackend>,
)> {
    let (run_reader, run_writer, configuration_reader, configuration_writer, target) =
        sessions.into_parts();
    let configuration =
        qualify_and_open_configuration_history(PostgresConfigurationHistoryBackend::from_sessions(
            configuration_reader,
            Some(configuration_writer),
            target.clone(),
        ))
        .await
        .map_err(|_| PostgresStoreError::Corruption("configuration semantic open failed"))?;
    let run_backend =
        PostgresStructuredHistoryBackend::from_run_parts(run_reader, run_writer, target);
    let assembled =
        qualify_and_open_structured_store(run_backend, registry, physical_binding_verifier)
            .await
            .map_err(|_| PostgresStoreError::Corruption("runtime semantic open failed"))?;
    Ok((assembled, configuration))
}

/// Opens the application boundary with structured RunHistory and resolve-only configuration.
pub async fn open_structured_authoritative_application(
    sessions: PostgresApplicationSessions,
    registry: CertifiedProgramRegistry,
    physical_binding_verifier: Arc<dyn PhysicalObligationChecker>,
) -> Result<(
    OpenedStructuredStore<PostgresStructuredHistoryBackend>,
    mfm_store::structured::ConfigurationHistoryReader<PostgresConfigurationHistoryBackend>,
)> {
    let (run_reader, run_writer, configuration_reader, target) = sessions.into_application_parts();
    let configuration =
        qualify_and_open_configuration_history(PostgresConfigurationHistoryBackend::from_sessions(
            configuration_reader,
            None,
            target.clone(),
        ))
        .await
        .map_err(|_| PostgresStoreError::Corruption("configuration semantic open failed"))?;
    let assembled = qualify_and_open_structured_store(
        PostgresStructuredHistoryBackend::from_run_parts(run_reader, run_writer, target),
        registry,
        physical_binding_verifier,
    )
    .await
    .map_err(|_| PostgresStoreError::Corruption("runtime semantic open failed"))?;
    let (_writer, reader) = configuration.into_authorities();
    Ok((assembled, reader))
}

/// Opens the separately held deployment-maintenance authority for append-only configuration.
pub async fn open_configuration_maintenance(
    sessions: PostgresConfigurationSessions,
) -> Result<mfm_store::structured::ConfigurationHistoryWriter<PostgresConfigurationHistoryBackend>>
{
    let (configuration_reader, configuration_writer, target) = sessions.into_parts();
    let configuration =
        qualify_and_open_configuration_history(PostgresConfigurationHistoryBackend::from_sessions(
            configuration_reader,
            Some(configuration_writer),
            target,
        ))
        .await
        .map_err(|_| PostgresStoreError::Corruption("configuration semantic open failed"))?;
    let (writer, _reader) = configuration.into_authorities();
    Ok(writer)
}
