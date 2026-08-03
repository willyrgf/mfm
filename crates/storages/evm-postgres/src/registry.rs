//! Separately credentialed wallet-activation registry authorities.

use std::str::FromStr;

use mfm_evm::{
    canonical_wallet_reference, WalletNonceDomain, WalletNonceDomainActivationAttestation,
    WalletNonceDomainActivationRecord, WalletNonceStoreIncarnation,
    WalletNonceStorePromotionAttestation, WalletNonceStoreSuccessor,
};
use sqlx::{PgConnection, PgPool, Row};

use crate::error::{PostgresEvmWalletError, Result};
use crate::provider::{
    fresh_application_marker, target_session_marker_lock_keys, ProviderRegistryMaintenance,
    ProviderTargetContext,
};
use crate::schema::{ACTIVATION_ADMIN_ROLE, ACTIVATION_PUBLIC_ROLE};
use crate::support::{
    canonical_json, decode_canonical, open_role_pool, parse_reference_text, reference_text,
    session_identity, transaction_id,
};
use crate::WalletAuthorityProviderClient;

// The lineage advisory lock is the registry linearization point. Each
// statement after acquiring it must observe the preceding lock holder's
// commit so expected activation/promotion contention is classified exactly.
const REGISTRY_TRANSACTION_ISOLATION: &str = "SET TRANSACTION ISOLATION LEVEL READ COMMITTED";

/// Opens the deployment-only compare-and-append activation-registry authority.
pub async fn open_activation_registry_admin(
    database_url: &str,
    expected_schema: &str,
    provider: WalletAuthorityProviderClient,
) -> Result<PostgresWalletActivationRegistryAdmin> {
    let maintenance = provider.open_registry_maintenance().await?;
    let pool = open_role_pool(database_url, expected_schema, ACTIVATION_ADMIN_ROLE).await?;
    Ok(PostgresWalletActivationRegistryAdmin {
        pool,
        schema_name: expected_schema.to_owned(),
        maintenance,
    })
}

/// Opens the deployment/offline read-only activation verifier.
pub async fn open_activation_registry_public(
    database_url: &str,
    expected_schema: &str,
) -> Result<PostgresWalletActivationRegistryPublic> {
    let pool = open_role_pool(database_url, expected_schema, ACTIVATION_PUBLIC_ROLE).await?;
    Ok(PostgresWalletActivationRegistryPublic { pool })
}

/// Deployment-only permanent activation and promotion authority.
pub struct PostgresWalletActivationRegistryAdmin {
    pool: PgPool,
    schema_name: String,
    maintenance: ProviderRegistryMaintenance,
}

/// Test-only activation issuance interruption point inside one SQL transaction.
#[cfg(feature = "parity-tests")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationIssuanceFaultPoint {
    /// Return after inserting the initial incarnation.
    AfterIncarnationInsert,
    /// Return after inserting the initial lineage head.
    AfterLineageHeadInsert,
    /// Return after inserting the domain activation.
    AfterDomainActivationInsert,
}

impl PostgresWalletActivationRegistryAdmin {
    /// Atomically creates or exactly resolves one domain activation while
    /// observing one serialized current store-incarnation head.
    pub async fn issue_domain_activation(
        &self,
        record: &WalletNonceDomainActivationRecord,
        proposed_current_incarnation: &WalletNonceStoreIncarnation,
    ) -> Result<WalletNonceDomainActivationAttestation> {
        self.issue_domain_activation_inner(record, proposed_current_incarnation, None)
            .await
    }

    /// Interrupts issuance at one exact internal statement for atomicity qualification.
    #[cfg(feature = "parity-tests")]
    pub async fn issue_domain_activation_with_fault(
        &self,
        record: &WalletNonceDomainActivationRecord,
        proposed_current_incarnation: &WalletNonceStoreIncarnation,
        fault: ActivationIssuanceFaultPoint,
    ) -> Result<WalletNonceDomainActivationAttestation> {
        self.issue_domain_activation_inner(record, proposed_current_incarnation, Some(fault))
            .await
    }

    async fn issue_domain_activation_inner(
        &self,
        record: &WalletNonceDomainActivationRecord,
        proposed_current_incarnation: &WalletNonceStoreIncarnation,
        #[cfg(feature = "parity-tests")] fault: Option<ActivationIssuanceFaultPoint>,
        #[cfg(not(feature = "parity-tests"))] _fault: Option<()>,
    ) -> Result<WalletNonceDomainActivationAttestation> {
        validate_issuance_input(
            record,
            proposed_current_incarnation,
            self.maintenance.registry_lineage_ref(),
        )?;
        let activation_record_ref = canonical_wallet_reference(record)
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let incarnation_ref = canonical_wallet_reference(proposed_current_incarnation)
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        if record.initial_store_incarnation_ref != incarnation_ref {
            return Err(PostgresEvmWalletError::PermanentConflict);
        }

        let record_json = canonical_json(record)?;
        let incarnation_json = canonical_json(proposed_current_incarnation)?;
        let activation_ref_text = reference_text(&activation_record_ref)?;
        let incarnation_ref_text = reference_text(&incarnation_ref)?;

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        sqlx::query(REGISTRY_TRANSACTION_ISOLATION)
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        let application_marker = fresh_application_marker()?;
        sqlx::query("SELECT pg_catalog.set_config('application_name', $1, FALSE)")
            .bind(&application_marker)
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        acquire_target_session_marker(&mut transaction, &application_marker).await?;
        let (database_oid, backend_pid) = session_identity(&mut transaction).await?;
        let transaction_id = transaction_id(&mut transaction).await?;
        let provider_context = ProviderTargetContext {
            database_oid,
            backend_pid,
            transaction_id: Some(transaction_id),
            snapshot_id: None,
            schema_name: self.schema_name.clone(),
            application_marker,
            store_incarnation: proposed_current_incarnation.clone(),
        };
        lock_lineage(&mut transaction, &record.wallet_nonce_store_lineage_id).await?;

        if let Some(existing) =
            resolve_domain(&mut *transaction, record.wallet_nonce_domain.as_str()).await?
        {
            if existing == *record {
                let qualified = resolve_activation_attestation(
                    &mut *transaction,
                    record.wallet_nonce_domain.as_str(),
                )
                .await?
                .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
                let issuance_lease = self
                    .maintenance
                    .prepare_issuance(record, proposed_current_incarnation, provider_context)
                    .await?;
                if issuance_lease.registry_issuance_ref() != &qualified.registry_issuance_ref {
                    return Err(PostgresEvmWalletError::PermanentConflict);
                }
                transaction
                    .commit()
                    .await
                    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
                issuance_lease.confirm(&qualified).await?;
                return Ok(qualified);
            }
            return Err(PostgresEvmWalletError::PermanentConflict);
        }

        let initialize_lineage = match resolve_lineage_head(
            &mut *transaction,
            &record.wallet_nonce_store_lineage_id,
            true,
        )
        .await?
        {
            Some((writer_epoch, current_ref)) => {
                if writer_epoch != proposed_current_incarnation.writer_epoch
                    || current_ref != incarnation_ref
                {
                    return Err(PostgresEvmWalletError::PermanentConflict);
                }
                let retained_json = sqlx::query_scalar::<_, String>(
                    "SELECT incarnation_json FROM wallet_store_incarnations \
                     WHERE wallet_nonce_store_lineage_id = $1 \
                       AND writer_epoch = $2::numeric",
                )
                .bind(&record.wallet_nonce_store_lineage_id)
                .bind(writer_epoch.to_string())
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
                if retained_json != incarnation_json {
                    return Err(PostgresEvmWalletError::PermanentConflict);
                }
                false
            }
            None => {
                if proposed_current_incarnation.writer_epoch != 1 {
                    return Err(PostgresEvmWalletError::PermanentConflict);
                }
                true
            }
        };

        let issuance_lease = self
            .maintenance
            .prepare_issuance(record, proposed_current_incarnation, provider_context)
            .await?;

        let qualified = WalletNonceDomainActivationAttestation {
            activation_record_ref: activation_record_ref.clone(),
            registry_issuance_ref: issuance_lease.registry_issuance_ref().clone(),
            activation_registry_lineage_ref: self.maintenance.registry_lineage_ref().clone(),
            initial_store_incarnation_ref: incarnation_ref.clone(),
            current_schema_record: record.clone(),
        };
        qualified
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;

        if initialize_lineage {
            sqlx::query(
                "INSERT INTO wallet_store_incarnations ( \
                     wallet_nonce_store_lineage_id, writer_epoch, incarnation_ref, \
                     predecessor_incarnation_ref, incarnation_json \
                 ) VALUES ($1, $2::numeric, $3, NULL, $4)",
            )
            .bind(&record.wallet_nonce_store_lineage_id)
            .bind(proposed_current_incarnation.writer_epoch.to_string())
            .bind(&incarnation_ref_text)
            .bind(&incarnation_json)
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::PermanentConflict)?;
            #[cfg(feature = "parity-tests")]
            if fault == Some(ActivationIssuanceFaultPoint::AfterIncarnationInsert) {
                return Err(PostgresEvmWalletError::Unavailable);
            }
            sqlx::query(
                "INSERT INTO wallet_store_lineage_heads ( \
                     wallet_nonce_store_lineage_id, current_writer_epoch, \
                     current_incarnation_ref \
                 ) VALUES ($1, $2::numeric, $3)",
            )
            .bind(&record.wallet_nonce_store_lineage_id)
            .bind(proposed_current_incarnation.writer_epoch.to_string())
            .bind(&incarnation_ref_text)
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::PermanentConflict)?;
            #[cfg(feature = "parity-tests")]
            if fault == Some(ActivationIssuanceFaultPoint::AfterLineageHeadInsert) {
                return Err(PostgresEvmWalletError::Unavailable);
            }
        }

        sqlx::query(
            "INSERT INTO wallet_domain_activations ( \
                 wallet_nonce_domain_id, activation_record_ref, \
                 wallet_nonce_store_lineage_id, observed_store_incarnation_ref, \
                 activation_record_json, registry_issuance_ref, activation_attestation_json \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(record.wallet_nonce_domain.as_str())
        .bind(&activation_ref_text)
        .bind(&record.wallet_nonce_store_lineage_id)
        .bind(&incarnation_ref_text)
        .bind(&record_json)
        .bind(reference_text(&qualified.registry_issuance_ref)?)
        .bind(canonical_json(&qualified)?)
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresEvmWalletError::PermanentConflict)?;
        #[cfg(feature = "parity-tests")]
        if fault == Some(ActivationIssuanceFaultPoint::AfterDomainActivationInsert) {
            return Err(PostgresEvmWalletError::Unavailable);
        }

        let committed = match transaction.commit().await {
            Ok(()) => qualified,
            Err(_) => self
                .resolve_domain_activation(&record.wallet_nonce_domain)
                .await?
                .filter(|existing| existing == &qualified)
                .ok_or(PostgresEvmWalletError::Unavailable)?,
        };
        issuance_lease.confirm(&committed).await?;
        Ok(committed)
    }

    /// Atomically advances one lineage by exact-head CAS after deployment has
    /// supplied all irreversible fence, prefix, and closed-replacement proofs.
    pub async fn promote_store_incarnation(
        &self,
        request: &WalletNonceStoreSuccessor,
    ) -> Result<WalletNonceStorePromotionAttestation> {
        request
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let request_ref = canonical_wallet_reference(request)
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let next_ref = canonical_wallet_reference(&request.next_incarnation)
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let request_ref_text = reference_text(&request_ref)?;

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        sqlx::query(REGISTRY_TRANSACTION_ISOLATION)
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        let application_marker = fresh_application_marker()?;
        sqlx::query("SELECT pg_catalog.set_config('application_name', $1, FALSE)")
            .bind(&application_marker)
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        acquire_target_session_marker(&mut transaction, &application_marker).await?;
        let (database_oid, backend_pid) = session_identity(&mut transaction).await?;
        let transaction_id = transaction_id(&mut transaction).await?;
        let provider_context = ProviderTargetContext {
            database_oid,
            backend_pid,
            transaction_id: Some(transaction_id),
            snapshot_id: None,
            schema_name: self.schema_name.clone(),
            application_marker,
            store_incarnation: request.next_incarnation.clone(),
        };
        lock_lineage(&mut transaction, &request.wallet_nonce_store_lineage_id).await?;

        let promotion_lease = self
            .maintenance
            .prepare_promotion(request, provider_context)
            .await?;

        if let Some(existing) = resolve_promotion(&mut *transaction, &request_ref_text).await? {
            transaction
                .commit()
                .await
                .map_err(|_| PostgresEvmWalletError::Unavailable)?;
            promotion_lease.confirm(&existing).await?;
            return Ok(existing);
        }

        let (writer_epoch, current_ref) = resolve_lineage_head(
            &mut *transaction,
            &request.wallet_nonce_store_lineage_id,
            true,
        )
        .await?
        .ok_or(PostgresEvmWalletError::PermanentConflict)?;
        let next_epoch = writer_epoch
            .checked_add(1)
            .ok_or(PostgresEvmWalletError::PermanentConflict)?;
        if current_ref != request.expected_current_incarnation_ref
            || request.next_incarnation.writer_epoch != next_epoch
            || next_ref == current_ref
        {
            return Err(PostgresEvmWalletError::PermanentConflict);
        }

        let proof = WalletNonceStorePromotionAttestation {
            wallet_nonce_store_lineage_id: request.wallet_nonce_store_lineage_id.clone(),
            previous_incarnation_ref: current_ref.clone(),
            current_incarnation_ref: next_ref.clone(),
            writer_epoch: next_epoch,
            successor_ref: request_ref.clone(),
            qualified_activation_registry_lineage_ref: self
                .maintenance
                .registry_lineage_ref()
                .clone(),
        };
        proof
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;

        sqlx::query(
            "INSERT INTO wallet_store_incarnations ( \
                 wallet_nonce_store_lineage_id, writer_epoch, incarnation_ref, \
                 predecessor_incarnation_ref, incarnation_json, promotion_successor_ref, \
                 promotion_attestation_json \
             ) VALUES ($1, $2::numeric, $3, $4, $5, $6, $7)",
        )
        .bind(&request.wallet_nonce_store_lineage_id)
        .bind(next_epoch.to_string())
        .bind(reference_text(&next_ref)?)
        .bind(reference_text(&current_ref)?)
        .bind(canonical_json(&request.next_incarnation)?)
        .bind(&request_ref_text)
        .bind(canonical_json(&proof)?)
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresEvmWalletError::PermanentConflict)?;
        let updated = sqlx::query(
            "UPDATE wallet_store_lineage_heads \
             SET current_writer_epoch = $2::numeric, current_incarnation_ref = $3 \
             WHERE wallet_nonce_store_lineage_id = $1 \
               AND current_writer_epoch = $4::numeric \
               AND current_incarnation_ref = $5",
        )
        .bind(&request.wallet_nonce_store_lineage_id)
        .bind(next_epoch.to_string())
        .bind(reference_text(&next_ref)?)
        .bind(writer_epoch.to_string())
        .bind(reference_text(&current_ref)?)
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        if updated.rows_affected() != 1 {
            return Err(PostgresEvmWalletError::PermanentConflict);
        }

        let committed = match transaction.commit().await {
            Ok(()) => proof,
            Err(_) => self
                .resolve_promotion(&request_ref)
                .await?
                .filter(|existing| existing == &proof)
                .ok_or(PostgresEvmWalletError::Unavailable)?,
        };
        promotion_lease.confirm(&committed).await?;
        Ok(committed)
    }

    /// Resolves one permanent domain activation without exposing its pool.
    pub async fn resolve_domain_activation(
        &self,
        domain: &WalletNonceDomain,
    ) -> Result<Option<WalletNonceDomainActivationAttestation>> {
        resolve_activation_attestation(&self.pool, domain.as_str()).await
    }

    /// Resolves one exact promotion proof after acknowledgement ambiguity.
    pub async fn resolve_promotion(
        &self,
        request_ref: &mfm_evm::EvmWalletReference,
    ) -> Result<Option<WalletNonceStorePromotionAttestation>> {
        resolve_promotion(&self.pool, &reference_text(request_ref)?).await
    }
}

/// Read-only registry proof resolver for deployment and offline verification.
pub struct PostgresWalletActivationRegistryPublic {
    pool: PgPool,
}

impl PostgresWalletActivationRegistryPublic {
    /// Reads and revalidates one permanent composite activation proof.
    pub async fn domain_activation(
        &self,
        domain: &WalletNonceDomain,
    ) -> Result<Option<WalletNonceDomainActivationAttestation>> {
        resolve_activation_attestation(&self.pool, domain.as_str()).await
    }

    /// Reads the current immutable incarnation object for one store lineage.
    pub async fn current_store_incarnation(
        &self,
        wallet_nonce_store_lineage_id: &str,
    ) -> Result<Option<WalletNonceStoreIncarnation>> {
        let row = sqlx::query(
            "SELECT i.incarnation_json, h.current_incarnation_ref \
             FROM wallet_store_lineage_heads h \
             JOIN wallet_store_incarnations i \
               ON i.wallet_nonce_store_lineage_id = h.wallet_nonce_store_lineage_id \
              AND i.writer_epoch = h.current_writer_epoch \
             WHERE h.wallet_nonce_store_lineage_id = $1",
        )
        .bind(wallet_nonce_store_lineage_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        row.map(|row| {
            let incarnation: WalletNonceStoreIncarnation = decode_canonical(
                row.try_get("incarnation_json")
                    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
            )?;
            incarnation
                .validate()
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
            let retained_ref = parse_reference_text(
                row.try_get("current_incarnation_ref")
                    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
            )?;
            if canonical_wallet_reference(&incarnation)
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?
                != retained_ref
            {
                return Err(PostgresEvmWalletError::InvalidAuthority);
            }
            Ok(incarnation)
        })
        .transpose()
    }
}

fn validate_issuance_input(
    record: &WalletNonceDomainActivationRecord,
    incarnation: &WalletNonceStoreIncarnation,
    registry_lineage_ref: &mfm_evm::EvmWalletReference,
) -> Result<()> {
    record
        .validate()
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    incarnation
        .validate()
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if record.wallet_nonce_store_lineage_id != incarnation.wallet_nonce_store_lineage_id
        || record.qualified_activation_registry_lineage_ref != *registry_lineage_ref
    {
        return Err(PostgresEvmWalletError::PermanentConflict);
    }
    Ok(())
}

async fn lock_lineage(
    connection: &mut PgConnection,
    wallet_nonce_store_lineage_id: &str,
) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(wallet_nonce_store_lineage_id)
        .execute(&mut *connection)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    Ok(())
}

async fn acquire_target_session_marker(connection: &mut PgConnection, marker: &str) -> Result<()> {
    let (first, second) = target_session_marker_lock_keys(marker)?;
    sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
        .bind(first)
        .bind(second)
        .execute(&mut *connection)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    Ok(())
}

async fn resolve_lineage_head<'e, E>(
    executor: E,
    wallet_nonce_store_lineage_id: &str,
    for_update: bool,
) -> Result<Option<(u64, mfm_evm::EvmWalletReference)>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = if for_update {
        "SELECT current_writer_epoch::text AS writer_epoch, current_incarnation_ref \
         FROM wallet_store_lineage_heads \
         WHERE wallet_nonce_store_lineage_id = $1 FOR UPDATE"
    } else {
        "SELECT current_writer_epoch::text AS writer_epoch, current_incarnation_ref \
         FROM wallet_store_lineage_heads WHERE wallet_nonce_store_lineage_id = $1"
    };
    let row = sqlx::query(sql)
        .bind(wallet_nonce_store_lineage_id)
        .fetch_optional(executor)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    row.map(|row| {
        let epoch = u64::from_str(
            row.try_get("writer_epoch")
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        )
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let reference = parse_reference_text(
            row.try_get("current_incarnation_ref")
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        )?;
        Ok((epoch, reference))
    })
    .transpose()
}

async fn resolve_domain<'e, E>(
    executor: E,
    domain_id: &str,
) -> Result<Option<WalletNonceDomainActivationRecord>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        "SELECT wallet_nonce_domain_id, wallet_nonce_store_lineage_id, activation_record_json \
           FROM wallet_domain_activations \
         WHERE wallet_nonce_domain_id = $1",
    )
    .bind(domain_id)
    .fetch_optional(executor)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    row.map(|row| {
        let record: WalletNonceDomainActivationRecord = decode_canonical(
            row.try_get("activation_record_json")
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        )?;
        record
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let retained_domain: &str = row
            .try_get("wallet_nonce_domain_id")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let retained_lineage: &str = row
            .try_get("wallet_nonce_store_lineage_id")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        if retained_domain != domain_id
            || retained_domain != record.wallet_nonce_domain.as_str()
            || retained_lineage != record.wallet_nonce_store_lineage_id
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(record)
    })
    .transpose()
}

async fn resolve_activation_attestation<'e, E>(
    executor: E,
    domain_id: &str,
) -> Result<Option<WalletNonceDomainActivationAttestation>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        "SELECT wallet_nonce_domain_id, activation_record_ref, \
                wallet_nonce_store_lineage_id, observed_store_incarnation_ref, \
                activation_record_json, registry_issuance_ref, activation_attestation_json \
         FROM wallet_domain_activations WHERE wallet_nonce_domain_id = $1",
    )
    .bind(domain_id)
    .fetch_optional(executor)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    row.map(|row| {
        let record: WalletNonceDomainActivationRecord = decode_canonical(
            row.try_get("activation_record_json")
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        )?;
        let qualified: WalletNonceDomainActivationAttestation = decode_canonical(
            row.try_get("activation_attestation_json")
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        )?;
        qualified
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let activation_ref = parse_reference_text(
            row.try_get("activation_record_ref")
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        )?;
        let issuance_ref = parse_reference_text(
            row.try_get("registry_issuance_ref")
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        )?;
        let observed_ref = parse_reference_text(
            row.try_get("observed_store_incarnation_ref")
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        )?;
        let retained_domain: &str = row
            .try_get("wallet_nonce_domain_id")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let retained_lineage: &str = row
            .try_get("wallet_nonce_store_lineage_id")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        if retained_domain != domain_id
            || retained_domain != record.wallet_nonce_domain.as_str()
            || qualified.current_schema_record.wallet_nonce_domain.as_str() != domain_id
            || retained_lineage != record.wallet_nonce_store_lineage_id
            || qualified.current_schema_record != record
            || qualified.activation_record_ref != activation_ref
            || qualified.registry_issuance_ref != issuance_ref
            || qualified.initial_store_incarnation_ref != observed_ref
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(qualified)
    })
    .transpose()
}

async fn resolve_promotion<'e, E>(
    executor: E,
    request_ref_text: &str,
) -> Result<Option<WalletNonceStorePromotionAttestation>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let json = sqlx::query_scalar::<_, String>(
        "SELECT promotion_attestation_json FROM wallet_store_incarnations \
         WHERE promotion_successor_ref = $1",
    )
    .bind(request_ref_text)
    .fetch_optional(executor)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    json.map(|json| {
        let proof: WalletNonceStorePromotionAttestation = decode_canonical(&json)?;
        proof
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        Ok(proof)
    })
    .transpose()
}
