use std::num::NonZeroU64;

use mfm_canonical::sha256_digest_bytes;
use mfm_evm::custody::{
    AuthorityError, AuthorityFuture, EvmTransactionAuthority, ExactRawTransaction,
    LoadedTransaction, NonceDomain, PreparedRecord, Reservation,
};
use mfm_evm::{EvmAddress, EvmAuthorityEpoch, EvmChainInstance, EvmHash};
use mfm_ids::{ContentDigest, ContentRef, EffectId, SchemaId};
use sqlx::PgConnection;

use crate::{GateError, PostgresEvmTransactionAuthority};

pub(crate) const EVM_TX_SCHEMA_CONTRACT: &str = "mfm.evm-transaction-postgres.v2";
pub(crate) const EVM_TX_SCHEMA_SQL: &str =
    include_str!("../migrations/evm_transaction_postgres_v2.sql");

impl EvmTransactionAuthority for PostgresEvmTransactionAuthority {
    fn authority_epoch(&self) -> &EvmAuthorityEpoch {
        &self.authority_epoch
    }

    fn load<'a>(
        &'a self,
        effect_id: &'a EffectId,
    ) -> AuthorityFuture<'a, Option<LoadedTransaction>> {
        Box::pin(async move {
            let mut connection = self.pool.acquire().await.map_err(unavailable)?;
            load_state(&mut connection, effect_id, &self.authority_epoch).await
        })
    }

    fn reserve_or_compare<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_value_ref: &'a ContentRef,
        domain: &'a NonceDomain,
        observed_pending_nonce: u64,
    ) -> AuthorityFuture<'a, Reservation> {
        Box::pin(async move {
            if domain.authority_epoch != self.authority_epoch {
                return Err(AuthorityError::Internal);
            }
            let mut transaction = begin_authority(&self.pool).await?;
            let lock_key = nonce_domain_lock_key(domain)?;
            sqlx::Executor::execute(
                &mut *transaction,
                sqlx::query!(
                    "SELECT pg_advisory_xact_lock($1) IS NULL AS locked",
                    lock_key,
                ),
            )
            .await
            .map_err(unavailable)?;

            if let Some(state) =
                load_state(&mut transaction, effect_id, &self.authority_epoch).await?
            {
                let reservation = &state.reservation;
                ensure_reservation(reservation, effect_id, command_value_ref, domain)?;
                let retained = reservation.clone();
                commit_authority(self, transaction).await?;
                return Ok(retained);
            }

            // The domain lock precedes this statement's READ COMMITTED snapshot.
            let local_next = match latest_reserved_nonce(&mut transaction, domain).await? {
                Some(nonce) => nonce.checked_add(1).ok_or(AuthorityError::Internal)?,
                None => 0,
            };
            let nonce = observed_pending_nonce.max(local_next);
            if nonce == u64::MAX {
                return Err(AuthorityError::Unavailable);
            }

            let inserted = insert_reservation(
                &mut transaction,
                effect_id,
                command_value_ref,
                domain,
                nonce,
            )
            .await?;
            let retained = if inserted {
                Reservation::new(
                    effect_id.clone(),
                    command_value_ref.clone(),
                    domain.clone(),
                    nonce,
                )
                .map_err(internal)?
            } else {
                let state = load_state(&mut transaction, effect_id, &self.authority_epoch)
                    .await?
                    .ok_or(AuthorityError::Internal)?;
                let reservation = &state.reservation;
                ensure_reservation(reservation, effect_id, command_value_ref, domain)?;
                reservation.clone()
            };
            commit_authority(self, transaction).await?;
            Ok(retained)
        })
    }

    fn retain_prepared<'a>(
        &'a self,
        reservation: &'a Reservation,
        candidate: &'a PreparedRecord,
    ) -> AuthorityFuture<'a, PreparedRecord> {
        Box::pin(async move {
            let effect_id = reservation.effect_id();
            let mut transaction = begin_authority(&self.pool).await?;
            let state = load_state(&mut transaction, effect_id, &self.authority_epoch)
                .await?
                .ok_or(AuthorityError::Internal)?;
            if &state.reservation != reservation {
                return Err(AuthorityError::Internal);
            }
            let retained = if let Some(prepared) = state.prepared {
                prepared
            } else {
                let hash = candidate.transaction_hash().as_bytes();
                sqlx::query!(
                    "INSERT INTO mfm_evm_tx.prepared_transactions (effect_id, \
                     transaction_hash, raw_transaction) VALUES ($1, $2, $3) ON \
                     CONFLICT DO NOTHING",
                    effect_id.as_str(),
                    hash.as_slice(),
                    candidate.raw_transaction().as_bytes(),
                )
                .execute(&mut *transaction)
                .await
                .map_err(unavailable)?;
                // ON CONFLICT may wait for a concurrent winner. This new statement sees it.
                load_state(&mut transaction, effect_id, &self.authority_epoch)
                    .await?
                    .ok_or(AuthorityError::Internal)?
                    .prepared
                    .ok_or(AuthorityError::Internal)?
            };
            commit_authority(self, transaction).await?;
            Ok(retained)
        })
    }
}

async fn begin_authority(
    pool: &sqlx::PgPool,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, AuthorityError> {
    let mut transaction = pool.begin().await.map_err(unavailable)?;
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL READ COMMITTED, READ WRITE")
        .execute(&mut *transaction)
        .await
        .map_err(unavailable)?;
    sqlx::query!("SET LOCAL synchronous_commit = on")
        .execute(&mut *transaction)
        .await
        .map_err(unavailable)?;
    Ok(transaction)
}

fn unavailable(_: impl Sized) -> AuthorityError {
    AuthorityError::Unavailable
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum AuthorityCommitFault {
    UnknownRolledBack,
    UnknownCommitted,
}

#[cfg(test)]
impl PostgresEvmTransactionAuthority {
    pub(crate) fn inject_authority_commit_fault(&self, fault: AuthorityCommitFault) {
        use std::sync::atomic::Ordering;

        let encoded = match fault {
            AuthorityCommitFault::UnknownRolledBack => 1,
            AuthorityCommitFault::UnknownCommitted => 2,
        };
        self.authority_commit_fault.store(encoded, Ordering::SeqCst);
    }
}

async fn commit_authority(
    backend: &PostgresEvmTransactionAuthority,
    transaction: sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AuthorityError> {
    #[cfg(test)]
    {
        use std::sync::atomic::Ordering;

        match backend.authority_commit_fault.swap(0, Ordering::SeqCst) {
            1 => {
                transaction.rollback().await.map_err(unavailable)?;
                return Err(AuthorityError::Unavailable);
            }
            2 => {
                transaction.commit().await.map_err(unavailable)?;
                return Err(AuthorityError::Unavailable);
            }
            _ => {}
        }
    }
    #[cfg(not(test))]
    let _ = backend;
    transaction.commit().await.map_err(unavailable)
}

fn ensure_reservation(
    reservation: &Reservation,
    effect_id: &EffectId,
    command_value_ref: &ContentRef,
    domain: &NonceDomain,
) -> Result<(), AuthorityError> {
    (reservation.effect_id() == effect_id
        && reservation.command_value_ref() == command_value_ref
        && reservation.domain() == domain)
        .then_some(())
        .ok_or(AuthorityError::Internal)
}

async fn load_state(
    connection: &mut PgConnection,
    effect_id: &EffectId,
    captured_epoch: &EvmAuthorityEpoch,
) -> Result<Option<LoadedTransaction>, AuthorityError> {
    let rows = sqlx::query!(
            "SELECT marker.schema_contract, marker.authority_epoch AS \
         admitted_epoch, reservation.effect_id, reservation.command_schema_id, \
         reservation.command_content_digest, reservation.authority_epoch AS \
         reservation_epoch, reservation.chain_id::text, reservation.genesis_hash, \
         reservation.sender, reservation.reserved_nonce::text, \
         prepared.transaction_hash, prepared.raw_transaction FROM ONLY mfm_evm_tx.mfm_evm_tx_schema AS \
         marker LEFT JOIN ONLY mfm_evm_tx.nonce_reservations AS reservation ON \
         reservation.effect_id = $1 LEFT JOIN ONLY \
         mfm_evm_tx.prepared_transactions AS prepared ON prepared.effect_id = \
         reservation.effect_id",
            effect_id.as_str(),
        )
    .fetch_all(connection)
    .await
    .map_err(unavailable)?;
    let [row]: [_; 1] = rows.try_into().map_err(internal)?;
    let admitted_epoch = epoch_from_bytes(&row.admitted_epoch)?;
    if row.schema_contract != EVM_TX_SCHEMA_CONTRACT || &admitted_epoch != captured_epoch {
        return Err(AuthorityError::Internal);
    }
    let reservation_absent = row.effect_id.is_none()
        && row.command_schema_id.is_none()
        && row.command_content_digest.is_none()
        && row.reservation_epoch.is_none()
        && row.chain_id.is_none()
        && row.genesis_hash.is_none()
        && row.sender.is_none()
        && row.reserved_nonce.is_none();
    if reservation_absent {
        return if row.transaction_hash.is_none() && row.raw_transaction.is_none() {
            Ok(None)
        } else {
            Err(AuthorityError::Internal)
        };
    }

    let (
        Some(retained_effect),
        Some(command_schema),
        Some(command_digest),
        Some(reservation_epoch),
        Some(chain_id),
        Some(genesis_hash),
        Some(sender),
        Some(reserved_nonce),
    ) = (
        row.effect_id,
        row.command_schema_id,
        row.command_content_digest,
        row.reservation_epoch,
        row.chain_id,
        row.genesis_hash,
        row.sender,
        row.reserved_nonce,
    )
    else {
        return Err(AuthorityError::Internal);
    };

    let retained_effect = EffectId::parse(retained_effect).map_err(internal)?;
    let command_value_ref = parse_content_ref(command_schema, command_digest)?;
    let epoch = epoch_from_bytes(&reservation_epoch)?;
    if epoch != admitted_epoch {
        return Err(AuthorityError::Internal);
    }
    let chain_id = NonZeroU64::new(parse_u64(&chain_id)?).ok_or(AuthorityError::Internal)?;
    let genesis = evm_hash_from_bytes(&genesis_hash)?;
    let sender = evm_address_from_bytes(&sender)?;
    let nonce = parse_u64(&reserved_nonce)?;
    let domain = NonceDomain {
        authority_epoch: epoch,
        chain_instance: EvmChainInstance {
            chain_id,
            expected_genesis_hash: genesis,
        },
        sender,
    };
    let reservation =
        Reservation::new(retained_effect, command_value_ref, domain, nonce).map_err(internal)?;
    let prepared = match (row.transaction_hash, row.raw_transaction) {
        (None, None) => None,
        (Some(hash), Some(raw)) => Some(PreparedRecord::new(
            evm_hash_from_bytes(&hash)?,
            ExactRawTransaction::new(raw)?,
        )),
        _ => return Err(AuthorityError::Internal),
    };
    Ok(Some(LoadedTransaction {
        reservation,
        prepared,
    }))
}

async fn latest_reserved_nonce(
    connection: &mut PgConnection,
    key: &NonceDomain,
) -> Result<Option<u64>, AuthorityError> {
    let epoch = key.authority_epoch.as_bytes();
    let genesis = key.chain_instance.expected_genesis_hash.as_bytes();
    let sender = key.sender.as_bytes();
    let retained: Option<String> = sqlx::query_scalar!(
        "SELECT reserved_nonce::text AS \"value!\" FROM ONLY mfm_evm_tx.nonce_reservations \
         WHERE authority_epoch = $1 AND chain_id = $2::text::numeric AND \
         genesis_hash = $3 AND sender = $4 ORDER BY reserved_nonce DESC LIMIT 1",
        epoch,
        key.chain_instance.chain_id.to_string(),
        genesis.as_slice(),
        sender.as_slice(),
    )
    .fetch_optional(connection)
    .await
    .map_err(unavailable)?;
    retained.map(|value| parse_u64(&value)).transpose()
}

async fn insert_reservation(
    connection: &mut PgConnection,
    effect_id: &EffectId,
    command_value_ref: &ContentRef,
    key: &NonceDomain,
    nonce: u64,
) -> Result<bool, AuthorityError> {
    let epoch = key.authority_epoch.as_bytes();
    let genesis = key.chain_instance.expected_genesis_hash.as_bytes();
    let sender = key.sender.as_bytes();
    let result = sqlx::query!(
        "INSERT INTO mfm_evm_tx.nonce_reservations (effect_id, \
         command_schema_id, command_content_digest, authority_epoch, chain_id, \
         genesis_hash, sender, reserved_nonce) VALUES ($1, $2, $3, $4, \
         $5::text::numeric, $6, $7, $8::text::numeric) ON CONFLICT DO NOTHING",
        effect_id.as_str(),
        command_value_ref.schema_id().as_str(),
        command_value_ref.content_digest().as_str(),
        epoch,
        key.chain_instance.chain_id.to_string(),
        genesis.as_slice(),
        sender.as_slice(),
        nonce.to_string(),
    )
    .execute(connection)
    .await
    .map_err(unavailable)?;
    Ok(result.rows_affected() == 1)
}

fn parse_content_ref(schema: String, digest: String) -> Result<ContentRef, AuthorityError> {
    ContentRef::new(
        SchemaId::parse(schema).map_err(internal)?,
        ContentDigest::parse(digest).map_err(internal)?,
    )
    .map_err(internal)
}

fn parse_u64(value: &str) -> Result<u64, AuthorityError> {
    if value.is_empty() || value.len() > 20 || value.starts_with('0') && value != "0" {
        return Err(AuthorityError::Internal);
    }
    value.parse().map_err(internal)
}

fn epoch_from_bytes(bytes: &[u8]) -> Result<EvmAuthorityEpoch, AuthorityError> {
    let exact: [u8; 32] = bytes.try_into().map_err(internal)?;
    Ok(EvmAuthorityEpoch::new(exact))
}

fn evm_hash_from_bytes(bytes: &[u8]) -> Result<EvmHash, AuthorityError> {
    let exact: [u8; 32] = bytes.try_into().map_err(internal)?;
    Ok(EvmHash::from_bytes(exact))
}

fn evm_address_from_bytes(bytes: &[u8]) -> Result<EvmAddress, AuthorityError> {
    let exact: [u8; 20] = bytes.try_into().map_err(internal)?;
    Ok(EvmAddress::from_bytes(exact))
}

fn nonce_domain_lock_key(key: &NonceDomain) -> Result<i64, AuthorityError> {
    let mut preimage = Vec::with_capacity(119);
    preimage.extend_from_slice(b"mfm.evm.nonce-domain-lock.v1\0");
    preimage.extend_from_slice(key.authority_epoch.as_bytes());
    preimage.extend_from_slice(&key.chain_instance.chain_id.get().to_be_bytes());
    preimage.extend_from_slice(key.chain_instance.expected_genesis_hash.as_bytes());
    preimage.extend_from_slice(key.sender.as_bytes());
    let digest = sha256_digest_bytes(&preimage);
    let first: [u8; 8] = digest.as_bytes()[..8].try_into().map_err(internal)?;
    Ok(i64::from_be_bytes(first))
}

fn internal(_: impl Sized) -> AuthorityError {
    AuthorityError::Internal
}

pub(crate) async fn load_evm_tx_epoch(
    connection: &mut PgConnection,
) -> Result<EvmAuthorityEpoch, GateError> {
    let markers: Vec<(String, Vec<u8>)> = sqlx::query!(
        "SELECT schema_contract AS \"schema_contract!\", authority_epoch AS \
         \"authority_epoch!\" FROM ONLY mfm_evm_tx.mfm_evm_tx_schema ORDER BY 1",
    )
    .fetch_all(&mut *connection)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|row| (row.schema_contract, row.authority_epoch))
            .collect()
    })
    .map_err(|error| {
        if crate::is_undefined_schema_object(&error) {
            GateError::Incompatible
        } else {
            GateError::Unavailable
        }
    })?;
    if markers.len() != 1 || markers[0].0 != EVM_TX_SCHEMA_CONTRACT {
        return Err(GateError::Incompatible);
    }
    epoch_from_bytes(&markers[0].1).map_err(|_| GateError::Incompatible)
}
