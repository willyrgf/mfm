//! Shared mechanical backend contract checks for Memory and durable adapters.

use std::sync::Arc;

use mfm_canonical::raw_content_digest;
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, RunId, SchemaId,
};

use crate::backend::{
    BackendAppendCommand, BackendAppendOutcome, BackendConfigurationOutcome, BackendError,
    ConfigurationAppendCommand, RawConfigurationRevision, RawFactPublication, RawFactSnapshot,
    RawFrameBytes, RawHistoryLoadLimit, RawRunPrefix, StructuredStoreBackend,
    StructuredStoreIdentity,
};

/// One transaction boundary exercised by the PostgreSQL fault probes.
#[derive(Clone, Copy)]
pub enum AtomicRollbackProbe {
    /// A history frame/head transaction.
    History,
    /// A history frame plus fact publication transaction.
    FactPublication,
    /// A configuration revision/head transaction.
    Configuration,
}

/// Exercises one append transaction with an externally installed SQL failpoint.
pub async fn exercise_atomic_rollback(
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
    probe: AtomicRollbackProbe,
) -> Result<(), BackendError> {
    let run_id = RunId::parse(match probe {
        AtomicRollbackProbe::History => {
            "run:sha256-jcs-v1:7123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
        AtomicRollbackProbe::FactPublication => {
            "run:sha256-jcs-v1:8123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
        AtomicRollbackProbe::Configuration => {
            "run:sha256-jcs-v1:9123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }
    })
    .map_err(|_| BackendError::Storage)?;
    let first_bytes = br#"{"kind":"rollback-first"}"#;
    let first_digest = raw_content_digest(first_bytes);
    let first_head = ContentDigest::parse(
        "content:sha256-v1:1111111111111111111111111111111111111111111111111111111111111111",
    )
    .map_err(|_| BackendError::Storage)?;
    let first_request = AppendRequestId::new("rollback-first-0123456789abcdef")
        .map_err(|_| BackendError::Storage)?;
    let first = BackendAppendCommand::new(
        &identity,
        &run_id,
        1,
        &first_request,
        first_bytes,
        &first_digest,
        &first_head,
        None,
        true,
        None,
    );

    match probe {
        AtomicRollbackProbe::History => {
            if backend.compare_and_append(&first).await.is_ok() {
                return Err(BackendError::Conflict);
            }
            let prefix = backend
                .load_complete_prefix(&run_id, RawHistoryLoadLimit::new(4, 4096))
                .await?;
            if prefix.is_some() {
                return Err(BackendError::Conflict);
            }
        }
        AtomicRollbackProbe::FactPublication => {
            if !matches!(
                backend.compare_and_append(&first).await?,
                BackendAppendOutcome::NewlyCommitted | BackendAppendOutcome::Found(_)
            ) {
                return Err(BackendError::Conflict);
            }
            let selection_schema = SchemaId::new(
                "mfm.test.rollback-selection",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([1; 32]),
            )
            .map_err(|_| BackendError::Storage)?;
            let selection_ref = ContentRef::new(selection_schema, raw_content_digest(b"selection"))
                .map_err(|_| BackendError::Storage)?;
            let publication = RawFactPublication::new(1, run_id.clone(), 2, selection_ref)?;
            let second_bytes = br#"{"kind":"rollback-second"}"#;
            let second_digest = raw_content_digest(second_bytes);
            let second_head = ContentDigest::parse(
                "content:sha256-v1:2222222222222222222222222222222222222222222222222222222222222222",
            )
            .map_err(|_| BackendError::Storage)?;
            let second_request = AppendRequestId::new("rollback-second-0123456789abcdef")
                .map_err(|_| BackendError::Storage)?;
            let second = BackendAppendCommand::new(
                &identity,
                &run_id,
                2,
                &second_request,
                second_bytes,
                &second_digest,
                &second_head,
                Some(&first_head),
                false,
                Some(&publication),
            );
            if backend.compare_and_append(&second).await.is_ok() {
                return Err(BackendError::Conflict);
            }
            let prefix = backend
                .load_complete_prefix(&run_id, RawHistoryLoadLimit::new(4, 4096))
                .await?
                .ok_or(BackendError::Storage)?;
            let fact_head = backend.load_facts().await?.head_sequence();
            if prefix.frames().len() != 1 || fact_head != 0 {
                return Err(BackendError::Conflict);
            }
        }
        AtomicRollbackProbe::Configuration => {
            let before = backend.load_configuration().await?;
            let bytes = br#"{"kind":"rollback-configuration"}"#;
            let schema = SchemaId::new(
                "mfm.test.rollback-configuration",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([2; 32]),
            )
            .map_err(|_| BackendError::Storage)?;
            let content_ref = ContentRef::new(schema, raw_content_digest(bytes))
                .map_err(|_| BackendError::Storage)?;
            let request = AppendRequestId::new("rollback-configuration-0123456789")
                .map_err(|_| BackendError::Storage)?;
            let command = ConfigurationAppendCommand::new(
                &identity,
                u64::try_from(before.len()).map_err(|_| BackendError::Capacity)?,
                &request,
                bytes,
                &content_ref,
            );
            if backend
                .compare_and_append_configuration(&command)
                .await
                .is_ok()
            {
                return Err(BackendError::Conflict);
            }
            if backend.load_configuration().await? != before {
                return Err(BackendError::Conflict);
            }
        }
    }
    Ok(())
}

/// Exercises the complete mechanical contract without decoding or reducing any frame.
pub async fn exercise(
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
) -> Result<(), BackendError> {
    if backend.identity() != identity {
        return Err(BackendError::Identity);
    }

    let run_id = RunId::parse(
        "run:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .map_err(|_| BackendError::Storage)?;
    let first_bytes = br#"{"kind":"conformance","sequence":1}"#;
    let first_digest = raw_content_digest(first_bytes);
    let first_head = ContentDigest::parse(
        "content:sha256-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .map_err(|_| BackendError::Storage)?;
    let first_request = AppendRequestId::new("backend-conformance-first-0123456789ab")
        .map_err(|_| BackendError::Storage)?;
    let first = BackendAppendCommand::new(
        &identity,
        &run_id,
        1,
        &first_request,
        first_bytes,
        &first_digest,
        &first_head,
        None,
        true,
        None,
    );
    assert_eq!(
        backend.compare_and_append(&first).await?,
        BackendAppendOutcome::NewlyCommitted
    );

    let retry = BackendAppendCommand::new(
        &identity,
        &run_id,
        1,
        &first_request,
        first_bytes,
        &first_digest,
        &first_head,
        None,
        true,
        None,
    );
    match backend.compare_and_append(&retry).await? {
        BackendAppendOutcome::Found(found) => {
            assert_eq!(found.sequence(), 1);
            assert_eq!(found.frame_bytes(), first_bytes);
        }
        other => panic!("unexpected idempotent backend outcome: {other:?}"),
    }

    let stale_request = AppendRequestId::new("backend-conformance-stale-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let stale = BackendAppendCommand::new(
        &identity,
        &run_id,
        1,
        &stale_request,
        first_bytes,
        &first_digest,
        &first_head,
        None,
        true,
        None,
    );
    assert!(matches!(
        backend.compare_and_append(&stale).await?,
        BackendAppendOutcome::StaleHead { actual_sequence: 1 }
    ));

    let second_bytes = br#"{"kind":"conformance","sequence":2}"#;
    let second_digest = raw_content_digest(second_bytes);
    let second_head = ContentDigest::parse(
        "content:sha256-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .map_err(|_| BackendError::Storage)?;
    let second_request = AppendRequestId::new("backend-conformance-race-a-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let second = BackendAppendCommand::new(
        &identity,
        &run_id,
        2,
        &second_request,
        second_bytes,
        &second_digest,
        &second_head,
        Some(&first_head),
        false,
        None,
    );
    let third_bytes = br#"{"kind":"conformance","sequence":3}"#;
    let third_digest = raw_content_digest(third_bytes);
    let third_head = ContentDigest::parse(
        "content:sha256-v1:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    )
    .map_err(|_| BackendError::Storage)?;
    let third_request = AppendRequestId::new("backend-conformance-race-b-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let third = BackendAppendCommand::new(
        &identity,
        &run_id,
        2,
        &third_request,
        third_bytes,
        &third_digest,
        &third_head,
        Some(&first_head),
        false,
        None,
    );
    let (left, right) = tokio::join!(
        backend.compare_and_append(&second),
        backend.compare_and_append(&third)
    );
    let left = left?;
    let right = right?;
    assert!(matches!(
        (&left, &right),
        (
            BackendAppendOutcome::NewlyCommitted,
            BackendAppendOutcome::StaleHead { actual_sequence: 2 }
        ) | (
            BackendAppendOutcome::StaleHead { actual_sequence: 2 },
            BackendAppendOutcome::NewlyCommitted
        )
    ));

    let prefix = backend
        .load_complete_prefix(&run_id, RawHistoryLoadLimit::new(4, 1024))
        .await?
        .ok_or(BackendError::Storage)?;
    assert_eq!(prefix.frames().len(), 2);
    assert_eq!(prefix.frames()[0].frame_digest(), &first_digest);

    let schema = SchemaId::new(
        "mfm.backend-conformance",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_hex("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .map_err(|_| BackendError::Storage)?,
    )
    .map_err(|_| BackendError::Storage)?;
    let configuration_bytes = br#"{"mode":"conformance"}"#;
    let configuration_ref = ContentRef::new(schema, raw_content_digest(configuration_bytes))
        .map_err(|_| BackendError::Storage)?;
    let configuration_request = AppendRequestId::new("backend-conformance-config-0123456789")
        .map_err(|_| BackendError::Storage)?;
    let configuration = ConfigurationAppendCommand::new(
        &identity,
        0,
        &configuration_request,
        configuration_bytes,
        &configuration_ref,
    );
    assert_eq!(
        backend
            .compare_and_append_configuration(&configuration)
            .await?,
        BackendConfigurationOutcome::NewlyCommitted
    );
    assert!(matches!(
        backend
            .compare_and_append_configuration(&configuration)
            .await?,
        BackendConfigurationOutcome::Found { sequence: 1 }
    ));

    let configuration_schema = SchemaId::new(
        "mfm.backend-conformance.configuration-race",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| BackendError::Storage)?;
    let left_configuration_bytes = br#"{"mode":"configuration-race-left"}"#;
    let left_configuration_ref = ContentRef::new(
        configuration_schema.clone(),
        raw_content_digest(left_configuration_bytes),
    )
    .map_err(|_| BackendError::Storage)?;
    let right_configuration_bytes = br#"{"mode":"configuration-race-right"}"#;
    let right_configuration_ref = ContentRef::new(
        configuration_schema,
        raw_content_digest(right_configuration_bytes),
    )
    .map_err(|_| BackendError::Storage)?;
    let left_configuration_request =
        AppendRequestId::new("backend-conformance-config-race-left-012345")
            .map_err(|_| BackendError::Storage)?;
    let right_configuration_request =
        AppendRequestId::new("backend-conformance-config-race-right-012345")
            .map_err(|_| BackendError::Storage)?;
    let left_configuration = ConfigurationAppendCommand::new(
        &identity,
        1,
        &left_configuration_request,
        left_configuration_bytes,
        &left_configuration_ref,
    );
    let right_configuration = ConfigurationAppendCommand::new(
        &identity,
        1,
        &right_configuration_request,
        right_configuration_bytes,
        &right_configuration_ref,
    );
    let (left, right) = tokio::join!(
        backend.compare_and_append_configuration(&left_configuration),
        backend.compare_and_append_configuration(&right_configuration)
    );
    let left = left?;
    let right = right?;
    assert!(matches!(
        (&left, &right),
        (
            BackendConfigurationOutcome::NewlyCommitted,
            BackendConfigurationOutcome::StaleHead { actual_sequence: 2 }
        ) | (
            BackendConfigurationOutcome::StaleHead { actual_sequence: 2 },
            BackendConfigurationOutcome::NewlyCommitted
        )
    ));
    let configuration_rows = backend.load_configuration().await?;
    assert_eq!(configuration_rows.len(), 2);
    assert_eq!(configuration_rows[0].canonical_bytes(), configuration_bytes);

    let facts = backend.load_facts().await?;
    assert_eq!(facts, RawFactSnapshot::new(0, Vec::new())?);
    let run_ids = backend.audit_run_ids().await?;
    assert_eq!(run_ids, vec![run_id]);

    assert!(RawFrameBytes::new(
        0,
        first_request,
        first_bytes.to_vec(),
        first_digest,
        first_head,
    )
    .is_err());
    assert!(RawRunPrefix::new(Vec::new()).is_err());
    assert!(RawConfigurationRevision::new(
        0,
        configuration_request,
        configuration_bytes.to_vec(),
        configuration_ref,
    )
    .is_err());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemoryStructuredBackend;
    use mfm_ids::{StoreEpoch, StoreScopeId, TenantScopeId};

    #[tokio::test]
    async fn memory_backend_satisfies_shared_contract() {
        let identity = StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            StoreEpoch::new(1),
            TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        );
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        exercise(backend, identity).await.expect("backend contract");
    }
}
