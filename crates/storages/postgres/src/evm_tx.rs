use mfm_canonical::{sha256_digest_bytes, CanonicalBytes, PlainCanonicalJsonBytes};
use mfm_evm::{EvmAddress, EvmAuthorityEpoch, EvmChainInstance, EvmHash, EvmTransactionSettlement};
use mfm_evm_transaction_authority::{
    AuthorityError, AuthorityFuture, AuthorityState, EvmTransactionAuthority, ExactRawTransaction,
    NonceDomain, PreparedRecord, Reservation, SettledRecord,
};
use mfm_ids::{ContentDigest, ContentRef, EffectId, SchemaId};
use mfm_values::canonicalize_mfm_value;
use sqlx::{PgConnection, Row};

use crate::{
    column, constraint, index, relation, runtime_table_privilege_mask, verify_schema_ownership,
    GateError, PostgresEvmTransactionAuthority, TABLE_INSERT, TABLE_SELECT,
};

pub(crate) const EVM_TX_SCHEMA_CONTRACT: &str = "mfm.evm-transaction-postgres.v2";
pub(crate) const EVM_TX_SCHEMA_SQL: &str =
    include_str!("../migrations/evm_transaction_postgres_v2.sql");
const MAX_SETTLEMENT_BYTES: usize = 65_536;

impl EvmTransactionAuthority for PostgresEvmTransactionAuthority {
    fn authority_epoch(&self) -> &EvmAuthorityEpoch {
        &self.authority_epoch
    }

    fn load<'a>(
        &'a self,
        effect_id: &'a EffectId,
        expected_command_ref: &'a ContentRef,
    ) -> AuthorityFuture<'a, Option<AuthorityState>> {
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(unavailable)?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *transaction)
                .await
                .map_err(unavailable)?;
            verify_captured_epoch(&mut transaction, &self.authority_epoch).await?;
            let retained = load_state(&mut transaction, effect_id).await?;
            if retained
                .as_ref()
                .is_some_and(|state| state_reservation(state).command_ref() != expected_command_ref)
            {
                return Err(AuthorityError::Internal);
            }
            transaction.commit().await.map_err(unavailable)?;
            Ok(retained)
        })
    }

    fn reserve_or_compare<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        domain: &'a NonceDomain,
        observed_pending_nonce: u64,
    ) -> AuthorityFuture<'a, Reservation> {
        Box::pin(async move {
            if domain.authority_epoch() != &self.authority_epoch {
                return Err(AuthorityError::Internal);
            }
            let mut transaction = self.pool.begin().await.map_err(unavailable)?;
            sqlx::query("SET LOCAL synchronous_commit = on")
                .execute(&mut *transaction)
                .await
                .map_err(unavailable)?;
            verify_captured_epoch(&mut transaction, &self.authority_epoch).await?;
            let lock_key = nonce_domain_lock_key(domain)?;
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(lock_key)
                .execute(&mut *transaction)
                .await
                .map_err(unavailable)?;

            if let Some(state) = load_state(&mut transaction, effect_id).await? {
                let reservation = state_reservation(&state);
                ensure_reservation(reservation, command_ref, domain)?;
                let retained = reservation.clone();
                commit_authority(self, transaction).await?;
                return Ok(retained);
            }

            let domain_exists = load_domain(&mut transaction, domain).await?;

            let nonce = match latest_reservation_effect(&mut transaction, domain).await? {
                None => observed_pending_nonce,
                Some(previous_effect_id) => {
                    let previous = load_state(&mut transaction, &previous_effect_id)
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

            if !domain_exists {
                insert_domain(&mut transaction, domain).await?;
            }
            let inserted =
                insert_reservation(&mut transaction, effect_id, command_ref, domain, nonce).await?;
            let retained = if inserted {
                Reservation::new(
                    effect_id.clone(),
                    command_ref.clone(),
                    domain.clone(),
                    nonce,
                )
            } else {
                let state = load_state(&mut transaction, effect_id)
                    .await?
                    .ok_or(AuthorityError::Unavailable)?;
                let reservation = state_reservation(&state);
                ensure_reservation(reservation, command_ref, domain)?;
                reservation.clone()
            };
            commit_authority(self, transaction).await?;
            Ok(retained)
        })
    }

    fn retain_prepared<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        transaction_hash: &'a EvmHash,
        raw_transaction: &'a ExactRawTransaction,
    ) -> AuthorityFuture<'a, PreparedRecord> {
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(unavailable)?;
            sqlx::query("SET LOCAL synchronous_commit = on")
                .execute(&mut *transaction)
                .await
                .map_err(unavailable)?;
            verify_captured_epoch(&mut transaction, &self.authority_epoch).await?;
            let state = load_state(&mut transaction, effect_id)
                .await?
                .ok_or(AuthorityError::Internal)?;
            let reservation = state_reservation(&state);
            if reservation.command_ref() != command_ref {
                return Err(AuthorityError::Internal);
            }
            if let Some(prepared) = state_prepared(&state) {
                ensure_prepared(prepared, transaction_hash, raw_transaction)?;
                let retained = prepared.clone();
                commit_authority(self, transaction).await?;
                return Ok(retained);
            }

            let transaction_hash_bytes = decode_hex::<32>(transaction_hash.as_str())?;
            let inserted = sqlx::query(
                "INSERT INTO mfm_evm_tx.prepared_transactions \
                 (effect_id, transaction_hash, raw_transaction) VALUES ($1, $2, $3) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(effect_id.as_str())
            .bind(transaction_hash_bytes.as_slice())
            .bind(raw_transaction.as_bytes())
            .execute(&mut *transaction)
            .await
            .map_err(unavailable)?
            .rows_affected()
                == 1;
            let retained = if inserted {
                PreparedRecord::new(
                    reservation.clone(),
                    transaction_hash.clone(),
                    raw_transaction.clone(),
                )
            } else {
                let state = load_state(&mut transaction, effect_id)
                    .await?
                    .ok_or(AuthorityError::Internal)?;
                let prepared = state_prepared(&state).ok_or(AuthorityError::Internal)?;
                ensure_prepared(prepared, transaction_hash, raw_transaction)?;
                prepared.clone()
            };
            commit_authority(self, transaction).await?;
            Ok(retained)
        })
    }

    fn retain_settlement<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        evidence: &'a EvmTransactionSettlement,
    ) -> AuthorityFuture<'a, SettledRecord> {
        Box::pin(async move {
            let (canonical, _) =
                canonicalize_mfm_value(evidence).map_err(|_| AuthorityError::Internal)?;
            if canonical.as_bytes().is_empty() || canonical.as_bytes().len() > MAX_SETTLEMENT_BYTES
            {
                return Err(AuthorityError::Internal);
            }
            let mut transaction = self.pool.begin().await.map_err(unavailable)?;
            sqlx::query("SET LOCAL synchronous_commit = on")
                .execute(&mut *transaction)
                .await
                .map_err(unavailable)?;
            verify_captured_epoch(&mut transaction, &self.authority_epoch).await?;
            let state = load_state(&mut transaction, effect_id)
                .await?
                .ok_or(AuthorityError::Internal)?;
            let reservation = state_reservation(&state);
            if reservation.command_ref() != command_ref || evidence.effect_id() != effect_id {
                return Err(AuthorityError::Internal);
            }
            let prepared = state_prepared(&state).ok_or(AuthorityError::Internal)?;
            let candidate = SettledRecord::new(prepared.clone(), evidence.clone())?;
            if let AuthorityState::Settled(retained) = &state {
                if retained != &candidate {
                    return Err(AuthorityError::Internal);
                }
                let retained = retained.clone();
                commit_authority(self, transaction).await?;
                return Ok(retained);
            }

            let inserted = sqlx::query(
                "INSERT INTO mfm_evm_tx.transaction_settlements (effect_id, settlement_bytes) \
                 VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(effect_id.as_str())
            .bind(canonical.as_bytes())
            .execute(&mut *transaction)
            .await
            .map_err(unavailable)?
            .rows_affected()
                == 1;
            let retained = if inserted {
                candidate
            } else {
                let AuthorityState::Settled(retained) = load_state(&mut transaction, effect_id)
                    .await?
                    .ok_or(AuthorityError::Internal)?
                else {
                    return Err(AuthorityError::Internal);
                };
                if retained != candidate {
                    return Err(AuthorityError::Internal);
                }
                retained
            };
            commit_authority(self, transaction).await?;
            Ok(retained)
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
    command_ref: &ContentRef,
    domain: &NonceDomain,
) -> Result<(), AuthorityError> {
    (reservation.command_ref() == command_ref && reservation.domain() == domain)
        .then_some(())
        .ok_or(AuthorityError::Internal)
}

fn ensure_prepared(
    prepared: &PreparedRecord,
    transaction_hash: &EvmHash,
    raw_transaction: &ExactRawTransaction,
) -> Result<(), AuthorityError> {
    (prepared.transaction_hash() == transaction_hash
        && prepared.raw_transaction() == raw_transaction)
        .then_some(())
        .ok_or(AuthorityError::Internal)
}

async fn verify_captured_epoch(
    connection: &mut PgConnection,
    captured: &EvmAuthorityEpoch,
) -> Result<(), AuthorityError> {
    let rows: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT authority_epoch FROM mfm_evm_tx.mfm_evm_tx_schema \
         WHERE schema_contract = $1",
    )
    .bind(EVM_TX_SCHEMA_CONTRACT)
    .fetch_all(connection)
    .await
    .map_err(unavailable)?;
    if rows.len() != 1 || epoch_from_bytes(&rows[0])? != *captured {
        return Err(AuthorityError::Internal);
    }
    Ok(())
}

async fn load_state(
    connection: &mut PgConnection,
    effect_id: &EffectId,
) -> Result<Option<AuthorityState>, AuthorityError> {
    let row = sqlx::query(
        "SELECT r.effect_id, r.command_schema_id, r.command_content_digest, \
                r.authority_epoch, r.chain_id::text, r.genesis_hash, r.sender, \
                r.reserved_nonce::text, \
                p.transaction_hash, p.raw_transaction, s.settlement_bytes \
         FROM mfm_evm_tx.nonce_reservations r \
         JOIN mfm_evm_tx.nonce_domains d \
           ON d.authority_epoch = r.authority_epoch AND d.chain_id = r.chain_id \
          AND d.genesis_hash = r.genesis_hash AND d.sender = r.sender \
         LEFT JOIN mfm_evm_tx.prepared_transactions p ON p.effect_id = r.effect_id \
         LEFT JOIN mfm_evm_tx.transaction_settlements s ON s.effect_id = r.effect_id \
         WHERE r.effect_id = $1",
    )
    .bind(effect_id.as_str())
    .fetch_optional(connection)
    .await
    .map_err(unavailable)?;
    row.map(parse_authority_state).transpose()
}

fn parse_authority_state(row: sqlx::postgres::PgRow) -> Result<AuthorityState, AuthorityError> {
    let retained_effect = EffectId::parse(row.try_get::<String, _>("effect_id").map_err(internal)?)
        .map_err(internal)?;
    let command_ref = parse_content_ref(
        row.try_get("command_schema_id").map_err(internal)?,
        row.try_get("command_content_digest").map_err(internal)?,
    )?;
    let epoch = epoch_from_bytes(
        &row.try_get::<Vec<u8>, _>("authority_epoch")
            .map_err(internal)?,
    )?;
    let chain_id = parse_u64(&row.try_get::<String, _>("chain_id").map_err(internal)?)?;
    let genesis = evm_hash_from_bytes(
        &row.try_get::<Vec<u8>, _>("genesis_hash")
            .map_err(internal)?,
    )?;
    let sender = evm_address_from_bytes(&row.try_get::<Vec<u8>, _>("sender").map_err(internal)?)?;
    let nonce = parse_u64(
        &row.try_get::<String, _>("reserved_nonce")
            .map_err(internal)?,
    )?;
    let domain = NonceDomain::new(
        epoch,
        EvmChainInstance::new(chain_id, genesis).map_err(internal)?,
        sender,
    );
    let reservation = Reservation::new(retained_effect, command_ref, domain, nonce);
    let hash: Option<Vec<u8>> = row.try_get("transaction_hash").map_err(internal)?;
    let raw: Option<Vec<u8>> = row.try_get("raw_transaction").map_err(internal)?;
    let settlement: Option<Vec<u8>> = row.try_get("settlement_bytes").map_err(internal)?;
    let (hash, raw) = match (hash, raw) {
        (None, None) if settlement.is_none() => return Ok(AuthorityState::Reserved(reservation)),
        (Some(hash), Some(raw)) => (hash, raw),
        _ => return Err(AuthorityError::Internal),
    };
    let prepared = PreparedRecord::new(
        reservation,
        evm_hash_from_bytes(&hash)?,
        ExactRawTransaction::new(raw)?,
    );
    let Some(settlement) = settlement else {
        return Ok(AuthorityState::Prepared(prepared));
    };
    let evidence = qualify_settlement(&settlement)?;
    Ok(AuthorityState::Settled(SettledRecord::new(
        prepared, evidence,
    )?))
}

async fn load_domain(
    connection: &mut PgConnection,
    key: &NonceDomain,
) -> Result<bool, AuthorityError> {
    let epoch = key.authority_epoch().as_bytes().map_err(internal)?;
    let genesis = decode_hex::<32>(key.chain_instance().expected_genesis_hash().as_str())?;
    let sender = decode_hex::<20>(key.sender().as_str())?;
    let retained: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM mfm_evm_tx.nonce_domains \
         WHERE authority_epoch = $1 AND chain_id = $2::numeric \
           AND genesis_hash = $3 AND sender = $4",
    )
    .bind(epoch.as_slice())
    .bind(key.chain_instance().chain_id().to_string())
    .bind(genesis.as_slice())
    .bind(sender.as_slice())
    .fetch_optional(connection)
    .await
    .map_err(unavailable)?;
    Ok(retained.is_some())
}

async fn latest_reservation_effect(
    connection: &mut PgConnection,
    key: &NonceDomain,
) -> Result<Option<EffectId>, AuthorityError> {
    let epoch = key.authority_epoch().as_bytes().map_err(internal)?;
    let genesis = decode_hex::<32>(key.chain_instance().expected_genesis_hash().as_str())?;
    let sender = decode_hex::<20>(key.sender().as_str())?;
    let retained: Option<String> = sqlx::query_scalar(
        "SELECT effect_id FROM mfm_evm_tx.nonce_reservations \
         WHERE authority_epoch = $1 AND chain_id = $2::numeric \
           AND genesis_hash = $3 AND sender = $4 \
         ORDER BY reserved_nonce DESC LIMIT 1",
    )
    .bind(epoch.as_slice())
    .bind(key.chain_instance().chain_id().to_string())
    .bind(genesis.as_slice())
    .bind(sender.as_slice())
    .fetch_optional(connection)
    .await
    .map_err(unavailable)?;
    retained
        .map(|value| EffectId::parse(value).map_err(internal))
        .transpose()
}

async fn insert_domain(
    connection: &mut PgConnection,
    domain: &NonceDomain,
) -> Result<(), AuthorityError> {
    let epoch = domain.authority_epoch().as_bytes().map_err(internal)?;
    let genesis = decode_hex::<32>(domain.chain_instance().expected_genesis_hash().as_str())?;
    let sender = decode_hex::<20>(domain.sender().as_str())?;
    let inserted = sqlx::query(
        "INSERT INTO mfm_evm_tx.nonce_domains \
         (authority_epoch, chain_id, genesis_hash, sender) \
         VALUES ($1, $2::numeric, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(epoch.as_slice())
    .bind(domain.chain_instance().chain_id().to_string())
    .bind(genesis.as_slice())
    .bind(sender.as_slice())
    .execute(&mut *connection)
    .await
    .map_err(unavailable)?
    .rows_affected();
    if inserted == 0 && !load_domain(connection, domain).await? {
        return Err(AuthorityError::Internal);
    }
    Ok(())
}

async fn insert_reservation(
    connection: &mut PgConnection,
    effect_id: &EffectId,
    command_ref: &ContentRef,
    key: &NonceDomain,
    nonce: u64,
) -> Result<bool, AuthorityError> {
    let epoch = key.authority_epoch().as_bytes().map_err(internal)?;
    let genesis = decode_hex::<32>(key.chain_instance().expected_genesis_hash().as_str())?;
    let sender = decode_hex::<20>(key.sender().as_str())?;
    let result = sqlx::query(
        "INSERT INTO mfm_evm_tx.nonce_reservations \
         (effect_id, command_schema_id, command_content_digest, authority_epoch, chain_id, \
          genesis_hash, sender, reserved_nonce) \
         VALUES ($1, $2, $3, $4, $5::numeric, $6, $7, $8::numeric) \
         ON CONFLICT DO NOTHING",
    )
    .bind(effect_id.as_str())
    .bind(command_ref.schema_id().as_str())
    .bind(command_ref.content_digest().as_str())
    .bind(epoch.as_slice())
    .bind(key.chain_instance().chain_id().to_string())
    .bind(genesis.as_slice())
    .bind(sender.as_slice())
    .bind(nonce.to_string())
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
    EvmHash::new(hex_prefixed(&exact)).map_err(internal)
}

fn evm_address_from_bytes(bytes: &[u8]) -> Result<EvmAddress, AuthorityError> {
    let exact: [u8; 20] = bytes.try_into().map_err(internal)?;
    EvmAddress::new(hex_prefixed(&exact)).map_err(internal)
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], AuthorityError> {
    let digits = value.strip_prefix("0x").ok_or(AuthorityError::Internal)?;
    if digits.len() != N * 2 {
        return Err(AuthorityError::Internal);
    }
    let mut output = [0_u8; N];
    for (index, pair) in digits.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(output)
}

fn hex_nibble(value: u8) -> Result<u8, AuthorityError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(AuthorityError::Internal),
    }
}

fn hex_prefixed(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(2 + bytes.len() * 2);
    output.push_str("0x");
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn nonce_domain_lock_preimage(key: &NonceDomain) -> Result<String, AuthorityError> {
    let epoch = key.authority_epoch().as_bytes().map_err(internal)?;
    let json = format!(
        "{{\"authority_epoch\":\"{}\",\"chain_id\":{},\"domain\":\"mfm.evm.nonce-domain-lock.v1\",\"expected_genesis_hash\":\"{}\",\"sender\":\"{}\"}}",
        CanonicalBytes::new(epoch.to_vec()).encoded(),
        key.chain_instance().chain_id(),
        key.chain_instance().expected_genesis_hash(),
        key.sender(),
    );
    PlainCanonicalJsonBytes::from_canonical_json_slice(json.as_bytes()).map_err(internal)?;
    Ok(json)
}

fn nonce_domain_lock_key(key: &NonceDomain) -> Result<i64, AuthorityError> {
    let preimage = nonce_domain_lock_preimage(key)?;
    let digest = sha256_digest_bytes(preimage.as_bytes());
    let first: [u8; 8] = digest.as_bytes()[..8].try_into().map_err(internal)?;
    Ok(i64::from_be_bytes(first))
}

fn internal(_: impl Sized) -> AuthorityError {
    AuthorityError::Internal
}

pub(crate) async fn verify_evm_tx_schema(
    connection: &mut PgConnection,
) -> Result<EvmAuthorityEpoch, GateError> {
    let relations: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT c.relname, c.relkind::text, c.relpersistence::text \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'mfm_evm_tx' AND c.relkind <> 'i' ORDER BY c.relname",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    if relations
        != [
            relation("mfm_evm_tx_schema", "r", "p"),
            relation("nonce_domains", "r", "p"),
            relation("nonce_reservations", "r", "p"),
            relation("prepared_transactions", "r", "p"),
            relation("transaction_settlements", "r", "p"),
        ]
    {
        return Err(GateError::Incompatible);
    }
    verify_schema_ownership(
        connection,
        "mfm_evm_tx",
        &[
            "mfm_evm_tx_schema",
            "nonce_domains",
            "nonce_reservations",
            "prepared_transactions",
            "transaction_settlements",
        ],
    )
    .await?;

    let columns: Vec<(String, String, String, bool, Option<String>)> = sqlx::query_as(
        "SELECT c.relname, a.attname, t.typname, a.attnotnull, coll.collname \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid \
         JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
         LEFT JOIN pg_catalog.pg_collation coll ON coll.oid = a.attcollation \
         WHERE n.nspname = 'mfm_evm_tx' AND c.relkind = 'r' \
           AND a.attnum > 0 AND NOT a.attisdropped \
         ORDER BY c.relname, a.attnum",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    let expected_columns = vec![
        column(
            "mfm_evm_tx_schema",
            "schema_contract",
            "text",
            true,
            Some("C"),
        ),
        column("mfm_evm_tx_schema", "authority_epoch", "bytea", true, None),
        column("nonce_domains", "authority_epoch", "bytea", true, None),
        column("nonce_domains", "chain_id", "numeric", true, None),
        column("nonce_domains", "genesis_hash", "bytea", true, None),
        column("nonce_domains", "sender", "bytea", true, None),
        column("nonce_reservations", "effect_id", "text", true, Some("C")),
        column(
            "nonce_reservations",
            "command_schema_id",
            "text",
            true,
            Some("C"),
        ),
        column(
            "nonce_reservations",
            "command_content_digest",
            "text",
            true,
            Some("C"),
        ),
        column("nonce_reservations", "authority_epoch", "bytea", true, None),
        column("nonce_reservations", "chain_id", "numeric", true, None),
        column("nonce_reservations", "genesis_hash", "bytea", true, None),
        column("nonce_reservations", "sender", "bytea", true, None),
        column(
            "nonce_reservations",
            "reserved_nonce",
            "numeric",
            true,
            None,
        ),
        column(
            "prepared_transactions",
            "effect_id",
            "text",
            true,
            Some("C"),
        ),
        column(
            "prepared_transactions",
            "transaction_hash",
            "bytea",
            true,
            None,
        ),
        column(
            "prepared_transactions",
            "raw_transaction",
            "bytea",
            true,
            None,
        ),
        column(
            "transaction_settlements",
            "effect_id",
            "text",
            true,
            Some("C"),
        ),
        column(
            "transaction_settlements",
            "settlement_bytes",
            "bytea",
            true,
            None,
        ),
    ];
    if columns != expected_columns {
        return Err(GateError::Incompatible);
    }
    let forbidden_shape: (i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM pg_catalog.pg_attrdef d \
             JOIN pg_catalog.pg_class c ON c.oid = d.adrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
            WHERE n.nspname = 'mfm_evm_tx'), \
           (SELECT count(*) FROM pg_catalog.pg_trigger t \
             JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
            WHERE n.nspname = 'mfm_evm_tx' AND NOT t.tgisinternal)",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    if forbidden_shape != (0, 0) {
        return Err(GateError::Incompatible);
    }

    let markers: Vec<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT schema_contract, authority_epoch FROM mfm_evm_tx.mfm_evm_tx_schema ORDER BY 1",
    )
    .fetch_all(&mut *connection)
    .await
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
    let epoch = epoch_from_bytes(&markers[0].1).map_err(|_| GateError::Incompatible)?;

    let indexes: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT table_class.relname, index_class.relname, pg_get_indexdef(index_class.oid) \
         FROM pg_catalog.pg_index idx \
         JOIN pg_catalog.pg_class table_class ON table_class.oid = idx.indrelid \
         JOIN pg_catalog.pg_class index_class ON index_class.oid = idx.indexrelid \
         JOIN pg_catalog.pg_namespace n ON n.oid = table_class.relnamespace \
         WHERE n.nspname = 'mfm_evm_tx' ORDER BY table_class.relname, index_class.relname",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    let expected_indexes = vec![
        index("mfm_evm_tx_schema", "mfm_evm_tx_schema_epoch_key", "CREATE UNIQUE INDEX mfm_evm_tx_schema_epoch_key ON mfm_evm_tx.mfm_evm_tx_schema USING btree (authority_epoch)"),
        index("mfm_evm_tx_schema", "mfm_evm_tx_schema_pkey", "CREATE UNIQUE INDEX mfm_evm_tx_schema_pkey ON mfm_evm_tx.mfm_evm_tx_schema USING btree (schema_contract)"),
        index("nonce_domains", "nonce_domains_pkey", "CREATE UNIQUE INDEX nonce_domains_pkey ON mfm_evm_tx.nonce_domains USING btree (authority_epoch, chain_id, genesis_hash, sender)"),
        index("nonce_reservations", "nonce_reservations_domain_nonce_key", "CREATE UNIQUE INDEX nonce_reservations_domain_nonce_key ON mfm_evm_tx.nonce_reservations USING btree (authority_epoch, chain_id, genesis_hash, sender, reserved_nonce)"),
        index("nonce_reservations", "nonce_reservations_pkey", "CREATE UNIQUE INDEX nonce_reservations_pkey ON mfm_evm_tx.nonce_reservations USING btree (effect_id)"),
        index("prepared_transactions", "prepared_transactions_pkey", "CREATE UNIQUE INDEX prepared_transactions_pkey ON mfm_evm_tx.prepared_transactions USING btree (effect_id)"),
        index("transaction_settlements", "transaction_settlements_pkey", "CREATE UNIQUE INDEX transaction_settlements_pkey ON mfm_evm_tx.transaction_settlements USING btree (effect_id)"),
    ];
    if indexes != expected_indexes {
        return Err(GateError::Incompatible);
    }

    let constraints: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT c.relname, con.conname, con.contype::text, pg_get_constraintdef(con.oid, false) \
         FROM pg_catalog.pg_constraint con \
         JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'mfm_evm_tx' AND con.contype IN ('c','f','p','u') \
         ORDER BY c.relname, con.conname",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    let expected_constraints = expected_evm_tx_constraints();
    if constraints != expected_constraints {
        return Err(GateError::Incompatible);
    }
    Ok(epoch)
}

fn expected_evm_tx_constraints() -> Vec<(String, String, String, String)> {
    vec![
        constraint("mfm_evm_tx_schema", "mfm_evm_tx_schema_contract_check", "c", "CHECK ((schema_contract = 'mfm.evm-transaction-postgres.v2'::text))"),
        constraint("mfm_evm_tx_schema", "mfm_evm_tx_schema_epoch_check", "c", "CHECK ((octet_length(authority_epoch) = 32))"),
        constraint("mfm_evm_tx_schema", "mfm_evm_tx_schema_epoch_key", "u", "UNIQUE (authority_epoch)"),
        constraint("mfm_evm_tx_schema", "mfm_evm_tx_schema_pkey", "p", "PRIMARY KEY (schema_contract)"),
        constraint("nonce_domains", "nonce_domains_chain_id_check", "c", "CHECK (((chain_id >= (1)::numeric) AND (chain_id <= '18446744073709551615'::numeric)))"),
        constraint("nonce_domains", "nonce_domains_epoch_check", "c", "CHECK ((octet_length(authority_epoch) = 32))"),
        constraint("nonce_domains", "nonce_domains_epoch_fkey", "f", "FOREIGN KEY (authority_epoch) REFERENCES mfm_evm_tx.mfm_evm_tx_schema(authority_epoch)"),
        constraint("nonce_domains", "nonce_domains_genesis_hash_check", "c", "CHECK ((octet_length(genesis_hash) = 32))"),
        constraint("nonce_domains", "nonce_domains_pkey", "p", "PRIMARY KEY (authority_epoch, chain_id, genesis_hash, sender)"),
        constraint("nonce_domains", "nonce_domains_sender_check", "c", "CHECK ((octet_length(sender) = 20))"),
        constraint("nonce_reservations", "nonce_reservations_chain_id_check", "c", "CHECK (((chain_id >= (1)::numeric) AND (chain_id <= '18446744073709551615'::numeric)))"),
        constraint("nonce_reservations", "nonce_reservations_command_digest_check", "c", "CHECK ((command_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'::text))"),
        constraint("nonce_reservations", "nonce_reservations_command_schema_id_check", "c", "CHECK ((((octet_length(command_schema_id) >= 1) AND (octet_length(command_schema_id) <= 512)) AND (command_schema_id ~ '^schema:[a-z0-9][a-z0-9._/-]*:[1-9][0-9]*:sha256-jcs-v1:[0-9a-f]{64}$'::text)))"),
        constraint("nonce_reservations", "nonce_reservations_domain_fkey", "f", "FOREIGN KEY (authority_epoch, chain_id, genesis_hash, sender) REFERENCES mfm_evm_tx.nonce_domains(authority_epoch, chain_id, genesis_hash, sender)"),
        constraint("nonce_reservations", "nonce_reservations_domain_nonce_key", "u", "UNIQUE (authority_epoch, chain_id, genesis_hash, sender, reserved_nonce)"),
        constraint("nonce_reservations", "nonce_reservations_effect_id_check", "c", "CHECK ((effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'::text))"),
        constraint("nonce_reservations", "nonce_reservations_epoch_check", "c", "CHECK ((octet_length(authority_epoch) = 32))"),
        constraint("nonce_reservations", "nonce_reservations_genesis_hash_check", "c", "CHECK ((octet_length(genesis_hash) = 32))"),
        constraint("nonce_reservations", "nonce_reservations_nonce_check", "c", "CHECK (((reserved_nonce >= (0)::numeric) AND (reserved_nonce <= '18446744073709551615'::numeric)))"),
        constraint("nonce_reservations", "nonce_reservations_pkey", "p", "PRIMARY KEY (effect_id)"),
        constraint("nonce_reservations", "nonce_reservations_sender_check", "c", "CHECK ((octet_length(sender) = 20))"),
        constraint("prepared_transactions", "prepared_transactions_effect_id_check", "c", "CHECK ((effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'::text))"),
        constraint("prepared_transactions", "prepared_transactions_hash_check", "c", "CHECK ((octet_length(transaction_hash) = 32))"),
        constraint("prepared_transactions", "prepared_transactions_pkey", "p", "PRIMARY KEY (effect_id)"),
        constraint("prepared_transactions", "prepared_transactions_raw_check", "c", "CHECK (((octet_length(raw_transaction) >= 1) AND (octet_length(raw_transaction) <= 132096)))"),
        constraint("prepared_transactions", "prepared_transactions_reservation_fkey", "f", "FOREIGN KEY (effect_id) REFERENCES mfm_evm_tx.nonce_reservations(effect_id)"),
        constraint("transaction_settlements", "transaction_settlements_bytes_check", "c", "CHECK (((octet_length(settlement_bytes) >= 1) AND (octet_length(settlement_bytes) <= 65536)))"),
        constraint("transaction_settlements", "transaction_settlements_effect_id_check", "c", "CHECK ((effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'::text))"),
        constraint("transaction_settlements", "transaction_settlements_pkey", "p", "PRIMARY KEY (effect_id)"),
        constraint("transaction_settlements", "transaction_settlements_prepared_fkey", "f", "FOREIGN KEY (effect_id) REFERENCES mfm_evm_tx.prepared_transactions(effect_id)"),
    ]
}

pub(crate) async fn verify_evm_tx_privileges(
    connection: &mut PgConnection,
) -> Result<bool, GateError> {
    for table in [
        "mfm_evm_tx.nonce_domains",
        "mfm_evm_tx.nonce_reservations",
        "mfm_evm_tx.prepared_transactions",
        "mfm_evm_tx.transaction_settlements",
    ] {
        if runtime_table_privilege_mask(connection, table).await? != TABLE_SELECT | TABLE_INSERT {
            return Ok(false);
        }
    }
    Ok(
        runtime_table_privilege_mask(connection, "mfm_evm_tx.mfm_evm_tx_schema").await?
            == TABLE_SELECT,
    )
}

#[cfg(test)]
pub(crate) fn test_lock_vector(
    key: &NonceDomain,
) -> Result<(String, [u8; 32], i64), AuthorityError> {
    let preimage = nonce_domain_lock_preimage(key)?;
    let digest = *sha256_digest_bytes(preimage.as_bytes()).as_bytes();
    Ok((preimage, digest, nonce_domain_lock_key(key)?))
}
