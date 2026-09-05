use std::num::NonZeroU64;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_evm::{EvmAddress, EvmAuthorityEpoch, EvmChainInstance, EvmHash, EvmTransactionSettlement};
use mfm_evm_transaction_authority::{
    AuthorityError, AuthorityFuture, AuthorityState, EvmTransactionAuthority, ExactRawTransaction,
    NonceDomain, PreparedRecord, Reservation, SettledRecord,
};
use mfm_ids::{ContentDigest, ContentRef, EffectId, SchemaId};
use mfm_values::canonicalize_mfm_value;
use sqlx::{PgConnection, Row};

use crate::{GateError, PostgresEvmTransactionAuthority};

pub(crate) const EVM_TX_SCHEMA_CONTRACT: &str = "mfm.evm-transaction-postgres.v1";
pub(crate) const EVM_TX_SCHEMA_SQL: &str =
    include_str!("../migrations/evm_transaction_postgres_v1.sql");
const MAX_SETTLEMENT_BYTES: usize = 65_536;

impl EvmTransactionAuthority for PostgresEvmTransactionAuthority {
    fn authority_epoch(&self) -> &EvmAuthorityEpoch {
        &self.authority_epoch
    }

    fn load<'a>(&'a self, effect_id: &'a EffectId) -> AuthorityFuture<'a, Option<AuthorityState>> {
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
            if domain.authority_epoch() != &self.authority_epoch {
                return Err(AuthorityError::Internal);
            }
            let mut transaction = self.pool.begin().await.map_err(unavailable)?;
            sqlx::query!("SET LOCAL synchronous_commit = on")
                .execute(&mut *transaction)
                .await
                .map_err(unavailable)?;
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
                let reservation = state_reservation(&state);
                ensure_reservation(reservation, effect_id, command_value_ref, domain)?;
                let retained = reservation.clone();
                commit_authority(self, transaction).await?;
                return Ok(retained);
            }

            let nonce = match latest_reservation_effect(&mut transaction, domain).await? {
                None => observed_pending_nonce,
                Some(previous_effect_id) => {
                    let previous =
                        load_state(&mut transaction, &previous_effect_id, &self.authority_epoch)
                            .await?
                            .ok_or(AuthorityError::Internal)?;
                    let AuthorityState::Settled(previous) = previous else {
                        return Err(AuthorityError::Unavailable);
                    };
                    let next = previous
                        .prepared()
                        .reservation()
                        .nonce()
                        .checked_add(1)
                        .ok_or(AuthorityError::Internal)?;
                    if observed_pending_nonce != next {
                        return Err(AuthorityError::Unavailable);
                    }
                    next
                }
            };

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
            } else {
                let state = load_state(&mut transaction, effect_id, &self.authority_epoch)
                    .await?
                    .ok_or(AuthorityError::Internal)?;
                let reservation = state_reservation(&state);
                ensure_reservation(reservation, effect_id, command_value_ref, domain)?;
                reservation.clone()
            };
            commit_authority(self, transaction).await?;
            Ok(retained)
        })
    }

    fn retain_prepared<'a>(&'a self, candidate: &'a PreparedRecord) -> AuthorityFuture<'a, ()> {
        Box::pin(async move {
            let effect_id = candidate.reservation().effect_id();
            let mut transaction = self.pool.begin().await.map_err(unavailable)?;
            sqlx::query!("SET LOCAL synchronous_commit = on")
                .execute(&mut *transaction)
                .await
                .map_err(unavailable)?;
            let state = load_state(&mut transaction, effect_id, &self.authority_epoch)
                .await?
                .ok_or(AuthorityError::Internal)?;
            let reservation = state_reservation(&state);
            if reservation != candidate.reservation() {
                return Err(AuthorityError::Internal);
            }
            if let Some(prepared) = state_prepared(&state) {
                if prepared != candidate {
                    return Err(AuthorityError::Unavailable);
                }
                commit_authority(self, transaction).await?;
                return Ok(());
            }

            let transaction_hash_bytes = candidate.transaction_hash().as_bytes();
            let inserted = sqlx::query!(
                "INSERT INTO mfm_evm_tx.prepared_transactions (effect_id, \
                 transaction_hash, raw_transaction) VALUES ($1, $2, $3) ON \
                 CONFLICT DO NOTHING",
                effect_id.as_str(),
                transaction_hash_bytes.as_slice(),
                candidate.raw_transaction().as_bytes(),
            )
            .execute(&mut *transaction)
            .await
            .map_err(unavailable)?
            .rows_affected()
                == 1;
            if !inserted {
                let state = load_state(&mut transaction, effect_id, &self.authority_epoch)
                    .await?
                    .ok_or(AuthorityError::Internal)?;
                if state_reservation(&state) != candidate.reservation() {
                    return Err(AuthorityError::Internal);
                }
                let prepared = state_prepared(&state).ok_or(AuthorityError::Internal)?;
                if prepared != candidate {
                    return Err(AuthorityError::Unavailable);
                }
            }
            commit_authority(self, transaction).await?;
            Ok(())
        })
    }

    fn retain_settlement<'a>(&'a self, candidate: &'a SettledRecord) -> AuthorityFuture<'a, ()> {
        Box::pin(async move {
            let effect_id = candidate.prepared().reservation().effect_id();
            let (canonical, _) = canonicalize_mfm_value(candidate.evidence())
                .map_err(|_| AuthorityError::Internal)?;
            if canonical.as_bytes().is_empty() || canonical.as_bytes().len() > MAX_SETTLEMENT_BYTES
            {
                return Err(AuthorityError::Internal);
            }
            let mut transaction = self.pool.begin().await.map_err(unavailable)?;
            sqlx::query!("SET LOCAL synchronous_commit = on")
                .execute(&mut *transaction)
                .await
                .map_err(unavailable)?;
            let state = load_state(&mut transaction, effect_id, &self.authority_epoch)
                .await?
                .ok_or(AuthorityError::Internal)?;
            let prepared = state_prepared(&state).ok_or(AuthorityError::Internal)?;
            if prepared != candidate.prepared() {
                return Err(AuthorityError::Internal);
            }
            if let AuthorityState::Settled(retained) = &state {
                if retained != candidate {
                    return Err(AuthorityError::Unavailable);
                }
                commit_authority(self, transaction).await?;
                return Ok(());
            }

            let inserted = sqlx::query!(
                "INSERT INTO mfm_evm_tx.transaction_settlements (effect_id, \
                 settlement_bytes) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                effect_id.as_str(),
                canonical.as_bytes(),
            )
            .execute(&mut *transaction)
            .await
            .map_err(unavailable)?
            .rows_affected()
                == 1;
            if !inserted {
                let AuthorityState::Settled(retained) =
                    load_state(&mut transaction, effect_id, &self.authority_epoch)
                        .await?
                        .ok_or(AuthorityError::Internal)?
                else {
                    return Err(AuthorityError::Internal);
                };
                if retained.prepared() != candidate.prepared() {
                    return Err(AuthorityError::Internal);
                }
                if &retained != candidate {
                    return Err(AuthorityError::Unavailable);
                }
            }
            commit_authority(self, transaction).await?;
            Ok(())
        })
    }
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

fn state_reservation(state: &AuthorityState) -> &Reservation {
    match state {
        AuthorityState::Reserved(reservation) => reservation,
        AuthorityState::Prepared(prepared) => prepared.reservation(),
        AuthorityState::Settled(settled) => settled.prepared().reservation(),
    }
}

fn state_prepared(state: &AuthorityState) -> Option<&PreparedRecord> {
    match state {
        AuthorityState::Reserved(_) => None,
        AuthorityState::Prepared(prepared) => Some(prepared),
        AuthorityState::Settled(settled) => Some(settled.prepared()),
    }
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
) -> Result<Option<AuthorityState>, AuthorityError> {
    let mut rows = sqlx::Executor::fetch_all(
        connection,
        sqlx::query!(
            "SELECT marker.schema_contract, marker.authority_epoch AS \
         admitted_epoch, reservation.effect_id, reservation.command_schema_id, \
         reservation.command_content_digest, reservation.authority_epoch AS \
         reservation_epoch, reservation.chain_id::text, reservation.genesis_hash, \
         reservation.sender, reservation.reserved_nonce::text, \
         prepared.transaction_hash, prepared.raw_transaction, \
         settled.settlement_bytes FROM ONLY mfm_evm_tx.mfm_evm_tx_schema AS \
         marker LEFT JOIN ONLY mfm_evm_tx.nonce_reservations AS reservation ON \
         reservation.effect_id = $1 LEFT JOIN ONLY \
         mfm_evm_tx.prepared_transactions AS prepared ON prepared.effect_id = \
         reservation.effect_id LEFT JOIN ONLY mfm_evm_tx.transaction_settlements \
         AS settled ON settled.effect_id = prepared.effect_id",
            effect_id.as_str(),
        ),
    )
    .await
    .map_err(unavailable)?;
    if rows.len() != 1 {
        return Err(AuthorityError::Internal);
    }
    parse_authority_state(rows.pop().ok_or(AuthorityError::Internal)?, captured_epoch)
}

fn parse_authority_state(
    row: sqlx::postgres::PgRow,
    captured_epoch: &EvmAuthorityEpoch,
) -> Result<Option<AuthorityState>, AuthorityError> {
    let schema_contract: String = row.try_get("schema_contract").map_err(internal)?;
    let admitted_epoch = epoch_from_bytes(
        &row.try_get::<Vec<u8>, _>("admitted_epoch")
            .map_err(internal)?,
    )?;
    if schema_contract != EVM_TX_SCHEMA_CONTRACT || &admitted_epoch != captured_epoch {
        return Err(AuthorityError::Internal);
    }

    let retained_effect: Option<String> = row.try_get("effect_id").map_err(internal)?;
    let command_schema: Option<String> = row.try_get("command_schema_id").map_err(internal)?;
    let command_digest: Option<String> = row.try_get("command_content_digest").map_err(internal)?;
    let reservation_epoch: Option<Vec<u8>> = row.try_get("reservation_epoch").map_err(internal)?;
    let chain_id: Option<String> = row.try_get("chain_id").map_err(internal)?;
    let genesis_hash: Option<Vec<u8>> = row.try_get("genesis_hash").map_err(internal)?;
    let sender: Option<Vec<u8>> = row.try_get("sender").map_err(internal)?;
    let reserved_nonce: Option<String> = row.try_get("reserved_nonce").map_err(internal)?;
    let hash: Option<Vec<u8>> = row.try_get("transaction_hash").map_err(internal)?;
    let raw: Option<Vec<u8>> = row.try_get("raw_transaction").map_err(internal)?;
    let settlement: Option<Vec<u8>> = row.try_get("settlement_bytes").map_err(internal)?;

    let reservation_absent = retained_effect.is_none()
        && command_schema.is_none()
        && command_digest.is_none()
        && reservation_epoch.is_none()
        && chain_id.is_none()
        && genesis_hash.is_none()
        && sender.is_none()
        && reserved_nonce.is_none();
    if reservation_absent {
        return if hash.is_none() && raw.is_none() && settlement.is_none() {
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
        retained_effect,
        command_schema,
        command_digest,
        reservation_epoch,
        chain_id,
        genesis_hash,
        sender,
        reserved_nonce,
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
    let domain = NonceDomain::new(epoch, EvmChainInstance::new(chain_id, genesis), sender);
    let reservation = Reservation::new(retained_effect, command_value_ref, domain, nonce);
    let (hash, raw) = match (hash, raw) {
        (None, None) if settlement.is_none() => {
            return Ok(Some(AuthorityState::Reserved(reservation)))
        }
        (Some(hash), Some(raw)) => (hash, raw),
        _ => return Err(AuthorityError::Internal),
    };
    let prepared = PreparedRecord::new(
        reservation,
        evm_hash_from_bytes(&hash)?,
        ExactRawTransaction::new(raw)?,
    );
    let Some(settlement) = settlement else {
        return Ok(Some(AuthorityState::Prepared(prepared)));
    };
    let evidence = qualify_settlement(&settlement)?;
    Ok(Some(AuthorityState::Settled(SettledRecord::new(
        prepared, evidence,
    )?)))
}

async fn latest_reservation_effect(
    connection: &mut PgConnection,
    key: &NonceDomain,
) -> Result<Option<EffectId>, AuthorityError> {
    let epoch = key.authority_epoch().as_bytes();
    let genesis = key.chain_instance().expected_genesis_hash().as_bytes();
    let sender = key.sender().as_bytes();
    let retained: Option<String> = sqlx::query_scalar!(
        "SELECT effect_id AS \"value!\" FROM ONLY mfm_evm_tx.nonce_reservations \
         WHERE authority_epoch = $1 AND chain_id = $2::text::numeric AND \
         genesis_hash = $3 AND sender = $4 ORDER BY reserved_nonce DESC LIMIT 1",
        epoch,
        key.chain_instance().chain_id().to_string(),
        genesis.as_slice(),
        sender.as_slice(),
    )
    .fetch_optional(connection)
    .await
    .map_err(unavailable)?;
    retained
        .map(|value| EffectId::parse(value).map_err(internal))
        .transpose()
}

async fn insert_reservation(
    connection: &mut PgConnection,
    effect_id: &EffectId,
    command_value_ref: &ContentRef,
    key: &NonceDomain,
    nonce: u64,
) -> Result<bool, AuthorityError> {
    let epoch = key.authority_epoch().as_bytes();
    let genesis = key.chain_instance().expected_genesis_hash().as_bytes();
    let sender = key.sender().as_bytes();
    let result = sqlx::query!(
        "INSERT INTO mfm_evm_tx.nonce_reservations (effect_id, \
         command_schema_id, command_content_digest, authority_epoch, chain_id, \
         genesis_hash, sender, reserved_nonce) VALUES ($1, $2, $3, $4, \
         $5::text::numeric, $6, $7, $8::text::numeric) ON CONFLICT DO NOTHING",
        effect_id.as_str(),
        command_value_ref.schema_id().as_str(),
        command_value_ref.content_digest().as_str(),
        epoch,
        key.chain_instance().chain_id().to_string(),
        genesis.as_slice(),
        sender.as_slice(),
        nonce.to_string(),
    )
    .execute(connection)
    .await
    .map_err(unavailable)?;
    Ok(result.rows_affected() == 1)
}

fn qualify_settlement(bytes: &[u8]) -> Result<EvmTransactionSettlement, AuthorityError> {
    if bytes.is_empty() || bytes.len() > MAX_SETTLEMENT_BYTES {
        return Err(AuthorityError::Internal);
    }
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes).map_err(internal)?;
    let evidence: EvmTransactionSettlement =
        serde_json::from_slice(canonical.as_bytes()).map_err(internal)?;
    let (recanonical, _) = canonicalize_mfm_value(&evidence).map_err(internal)?;
    (recanonical.as_bytes() == bytes)
        .then_some(evidence)
        .ok_or(AuthorityError::Internal)
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
    preimage.extend_from_slice(key.authority_epoch().as_bytes());
    preimage.extend_from_slice(&key.chain_instance().chain_id().get().to_be_bytes());
    preimage.extend_from_slice(key.chain_instance().expected_genesis_hash().as_bytes());
    preimage.extend_from_slice(key.sender().as_bytes());
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
