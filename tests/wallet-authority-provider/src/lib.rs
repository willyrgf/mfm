#![warn(missing_docs)]
//! Separate-process wallet-authority provider used only by qualification tests.
//!
//! The production repository owns only the authenticated client and enforcement
//! port. This test-layer package supplies a live process that exercises that
//! port while keeping its signing key and target-probe credential out of the
//! application and worker processes.

/// PostgreSQL commit-fault proxy used by live acknowledgement-ambiguity tests.
pub mod postgres_proxy;

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader as SyncBufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener as SyncUnixListener, UnixStream as SyncUnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mfm_evm::{
    canonical_wallet_reference, ActivateEvmCandidateRequest, ActiveWalletCandidate,
    CompleteEvmNonceRequest, CompletedWalletNonce, EvmRoutingCatalogDescriptor, EvmWalletReference,
    ReserveEvmNonceRequest, ReservedWalletNonce, WalletNonceDomainActivationAttestation,
    WalletNonceDomainActivationRecord, WalletNonceStoreIncarnation, WalletNonceStoreLineageHead,
    WalletNonceStorePromotionAttestation, WalletNonceStoreSuccessor,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId};
use mfm_journal::structured::{HistoryObject, LexicalValueRef};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::{AssertSqlSafe, PgPool, Row};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;
use zeroize::{Zeroize, Zeroizing};

const PROTOCOL_VERSION: u16 = 3;
const MAX_MESSAGE_BYTES: usize = 1_048_576;
const AUTHENTICATION_DOMAIN: &[u8] = b"mfm.wallet-authority-provider.authentication.v1\0";
const ASSERTION_DOMAIN: &[u8] = b"mfm.wallet-authority-provider.assertion.v1\0";
const CHECKPOINT_IO_TIMEOUT: Duration = Duration::from_secs(5);
const LEASE_HOLD_FAULT_DURATION: Duration = Duration::from_secs(2);
const DEPLOYMENT_ASSEMBLY_LEASE_TTL: Duration = Duration::from_secs(30);
const MAX_DEPLOYMENT_ASSEMBLY_LEASES: usize = 64;

static CHECKPOINT_SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Redaction-safe test-provider failure.
#[derive(Debug, thiserror::Error)]
pub enum ProviderTestError {
    /// The provider process or channel was unavailable.
    #[error("wallet authority test provider is unavailable")]
    Unavailable,
    /// Provider configuration or protocol data was invalid.
    #[error("wallet authority test provider contract is invalid")]
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointTarget {
    database_oid: u32,
    schema_name: String,
    store_incarnation: WalletNonceStoreIncarnation,
    current_public_lineage_head: HistoryObject,
    provider_fence_head_ref: EvmWalletReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
// Mutation transport owns this exact flat wire value behind a Box; the issuance and promotion
// variants remain bounded test-control values, so field-level boxes would only add allocations.
#[allow(clippy::large_enum_variant)]
enum ProviderMutation {
    ActivationIssuance {
        record: WalletNonceDomainActivationRecord,
        incarnation: WalletNonceStoreIncarnation,
        attestation: WalletNonceDomainActivationAttestation,
    },
    Reservation {
        request: ReserveEvmNonceRequest,
        reservation: ReservedWalletNonce,
        state_input_ref: LexicalValueRef,
    },
    CandidateActivation {
        request: ActivateEvmCandidateRequest,
        candidate: ActiveWalletCandidate,
        state_input_ref: LexicalValueRef,
    },
    Completion {
        request: CompleteEvmNonceRequest,
        completion: CompletedWalletNonce,
        state_input_ref: LexicalValueRef,
    },
    Promotion {
        successor: WalletNonceStoreSuccessor,
        attestation: WalletNonceStorePromotionAttestation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreIncarnationCheckpointRow {
    wallet_nonce_store_lineage_id: String,
    writer_epoch: String,
    incarnation_ref: String,
    predecessor_incarnation_ref: Option<String>,
    incarnation_json: String,
    promotion_successor_ref: Option<String>,
    promotion_attestation_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreLineageHeadCheckpointRow {
    wallet_nonce_store_lineage_id: String,
    current_writer_epoch: String,
    current_incarnation_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivationCheckpointRow {
    wallet_nonce_domain_id: String,
    activation_record_ref: String,
    wallet_nonce_store_lineage_id: String,
    observed_store_incarnation_ref: String,
    activation_record_json: String,
    registry_issuance_ref: String,
    activation_attestation_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DomainCheckpointRow {
    wallet_nonce_domain_id: String,
    activation_record_ref: String,
    wallet_nonce_store_lineage_id: String,
    activation_record_json: String,
    activation_attestation_json: String,
    local_high_water_nonce: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReservationCheckpointRow {
    semantic_reservation_key: String,
    wallet_nonce_domain_id: String,
    submission_intent_id: String,
    nonce: String,
    transaction_intent_digest: String,
    candidate_family_ref: String,
    request_json: String,
    transaction_intent_json: String,
    candidate_family_json: String,
    reservation_json: String,
    state_input_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateCheckpointRow {
    semantic_candidate_operation_key: String,
    semantic_reservation_key: String,
    candidate_ordinal: i32,
    request_json: String,
    active_candidate_json: String,
    state_input_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionCheckpointRow {
    semantic_completion_key: String,
    semantic_reservation_key: String,
    request_json: String,
    canonical_terminal_outcome_json: String,
    completion_json: String,
    state_input_json: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WalletCheckpointPrefix {
    store_incarnations: BTreeMap<String, StoreIncarnationCheckpointRow>,
    store_lineage_heads: BTreeMap<String, StoreLineageHeadCheckpointRow>,
    domain_activations: BTreeMap<String, ActivationCheckpointRow>,
    domains: BTreeMap<String, DomainCheckpointRow>,
    reservations: BTreeMap<String, ReservationCheckpointRow>,
    candidates: BTreeMap<String, CandidateCheckpointRow>,
    completions: BTreeMap<String, CompletionCheckpointRow>,
}

impl WalletCheckpointPrefix {
    fn digest(&self) -> Result<String, ProviderTestError> {
        mfm_journal::structured::domain_content_digest(
            "mfm.wallet-authority-provider.checkpoint-prefix.v1",
            self,
        )
        .map(|digest| digest.as_str().to_owned())
        .map_err(|_| ProviderTestError::Invalid)
    }

    fn exact_successor(mut self, mutation: &ProviderMutation) -> Result<Self, ProviderTestError> {
        apply_checkpoint_mutation(&mut self, mutation)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedCheckpoint {
    epoch: u64,
    predecessor_digest: String,
    successor_digest: String,
    operation_digest: String,
    successor_target: Option<CheckpointTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CheckpointRequest {
    Verify {
        fence_lineage_ref: EvmWalletReference,
        target: CheckpointTarget,
        observed_prefix_digest: String,
    },
    Recover {
        fence_lineage_ref: EvmWalletReference,
        target: CheckpointTarget,
        observed_prefix_digest: String,
    },
    Prepare {
        fence_lineage_ref: EvmWalletReference,
        target: CheckpointTarget,
        expected_epoch: u64,
        prepared: PreparedCheckpoint,
    },
    Acknowledge {
        fence_lineage_ref: EvmWalletReference,
        target: CheckpointTarget,
        prepared: PreparedCheckpoint,
        observed_prefix_digest: String,
    },
    Abort {
        fence_lineage_ref: EvmWalletReference,
        target: CheckpointTarget,
        prepared: PreparedCheckpoint,
        observed_prefix_digest: String,
    },
    Shutdown,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedCheckpointRequest {
    authentication_tag_hex: String,
    request: CheckpointRequest,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CheckpointReply {
    Current { epoch: u64 },
    Rejected,
}

struct CheckpointAuthorityState {
    authentication_token: Zeroizing<[u8; 32]>,
    fence_lineage_ref: EvmWalletReference,
    target: CheckpointTarget,
    epoch: u64,
    acknowledged_prefix_digest: String,
    prepared: Option<PreparedCheckpoint>,
}

/// Parent-process authority retaining the non-rollback checkpoint across child restarts.
pub struct ProviderCheckpointAuthority {
    endpoint: PathBuf,
    authentication_token: Zeroizing<[u8; 32]>,
    thread: Option<JoinHandle<()>>,
}

impl ProviderCheckpointAuthority {
    /// Starts a target-bound monotonic checkpoint authority from the exact current SQL prefix.
    pub async fn start(
        activation_database_url: &str,
        nonce_database_url: &str,
        schema_name: &str,
        fence_lineage_ref: EvmWalletReference,
        store_incarnation: WalletNonceStoreIncarnation,
        current_public_lineage_head: HistoryObject,
        provider_fence_head_ref: EvmWalletReference,
    ) -> Result<Self, ProviderTestError> {
        let activation_pool = PgPool::connect(activation_database_url)
            .await
            .map_err(|_| ProviderTestError::Unavailable)?;
        let nonce_pool = PgPool::connect(nonce_database_url)
            .await
            .map_err(|_| ProviderTestError::Unavailable)?;
        let database_oid = current_database_oid(&activation_pool).await?;
        if current_database_oid(&nonce_pool).await? != database_oid {
            return Err(ProviderTestError::Invalid);
        }
        // Initialization happens before any provider is ready, so the role-split
        // observation cannot straddle an admitted wallet mutation.
        let acknowledged_prefix_digest =
            wallet_checkpoint_prefix(&activation_pool, &nonce_pool, schema_name)
                .await?
                .digest()?;
        activation_pool.close().await;
        nonce_pool.close().await;
        let endpoint = std::env::temp_dir().join(format!(
            "mfm-wallet-checkpoint-{}-{}.sock",
            std::process::id(),
            CHECKPOINT_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        if endpoint.exists() {
            return Err(ProviderTestError::Invalid);
        }
        let listener =
            SyncUnixListener::bind(&endpoint).map_err(|_| ProviderTestError::Unavailable)?;
        std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| ProviderTestError::Unavailable)?;
        let authentication_token = random_bytes::<32>()?;
        let state = CheckpointAuthorityState {
            authentication_token: Zeroizing::new(*authentication_token),
            fence_lineage_ref,
            target: CheckpointTarget {
                database_oid,
                schema_name: schema_name.to_owned(),
                store_incarnation,
                current_public_lineage_head,
                provider_fence_head_ref,
            },
            epoch: 0,
            acknowledged_prefix_digest,
            prepared: None,
        };
        let thread = std::thread::Builder::new()
            .name("wallet-checkpoint-authority".to_owned())
            .spawn(move || run_checkpoint_authority(listener, state))
            .map_err(|_| ProviderTestError::Unavailable)?;
        Ok(Self {
            endpoint,
            authentication_token,
            thread: Some(thread),
        })
    }

    /// Returns the private Unix endpoint supplied to child provider processes.
    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }
}

impl Drop for ProviderCheckpointAuthority {
    fn drop(&mut self) {
        if let Ok(mut stream) = SyncUnixStream::connect(&self.endpoint) {
            let request = authenticate_checkpoint_request(
                self.authentication_token.as_ref(),
                CheckpointRequest::Shutdown,
            );
            if request
                .and_then(|request| {
                    serde_json::to_writer(&mut stream, &request)
                        .map_err(|_| ProviderTestError::Invalid)
                })
                .is_ok()
            {
                let _ = stream.write_all(b"\n");
                let _ = stream.flush();
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.endpoint);
    }
}

fn run_checkpoint_authority(listener: SyncUnixListener, mut state: CheckpointAuthorityState) {
    for accepted in listener.incoming() {
        let Ok(mut stream) = accepted else {
            break;
        };
        let _ = stream.set_read_timeout(Some(CHECKPOINT_IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(CHECKPOINT_IO_TIMEOUT));
        let Ok(read_stream) = stream.try_clone() else {
            continue;
        };
        let mut reader = SyncBufReader::new(read_stream);
        let request = read_bounded_sync_frame(&mut reader, MAX_MESSAGE_BYTES)
            .ok()
            .and_then(|frame| serde_json::from_slice::<AuthenticatedCheckpointRequest>(&frame).ok())
            .and_then(|envelope| authenticate_checkpoint_envelope(&state, envelope).ok());
        if matches!(request, Some(CheckpointRequest::Shutdown)) {
            break;
        }
        let reply = request
            .and_then(|request| apply_checkpoint_request(&mut state, request).ok())
            .unwrap_or(CheckpointReply::Rejected);
        if serde_json::to_writer(&mut stream, &reply).is_ok() {
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
        }
    }
}

fn authenticate_checkpoint_request(
    authentication_token: &[u8],
    request: CheckpointRequest,
) -> Result<AuthenticatedCheckpointRequest, ProviderTestError> {
    let encoded = serde_json::to_vec(&request).map_err(|_| ProviderTestError::Invalid)?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, authentication_token);
    let authentication_tag_hex = hex::encode(hmac::sign(&key, &encoded).as_ref());
    Ok(AuthenticatedCheckpointRequest {
        authentication_tag_hex,
        request,
    })
}

fn authenticate_checkpoint_envelope(
    state: &CheckpointAuthorityState,
    mut envelope: AuthenticatedCheckpointRequest,
) -> Result<CheckpointRequest, ProviderTestError> {
    let mut tag = Zeroizing::new([0_u8; 32]);
    hex::decode_to_slice(&envelope.authentication_tag_hex, tag.as_mut())
        .map_err(|_| ProviderTestError::Invalid)?;
    envelope.authentication_tag_hex.zeroize();
    let encoded = serde_json::to_vec(&envelope.request).map_err(|_| ProviderTestError::Invalid)?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, state.authentication_token.as_ref());
    hmac::verify(&key, &encoded, tag.as_ref()).map_err(|_| ProviderTestError::Invalid)?;
    Ok(envelope.request)
}

fn apply_checkpoint_request(
    state: &mut CheckpointAuthorityState,
    request: CheckpointRequest,
) -> Result<CheckpointReply, ProviderTestError> {
    match request {
        CheckpointRequest::Verify {
            fence_lineage_ref,
            target,
            observed_prefix_digest,
        } => {
            if fence_lineage_ref != state.fence_lineage_ref {
                return Err(ProviderTestError::Invalid);
            }
            if let Some(prepared) = state.prepared.clone() {
                if observed_prefix_digest == prepared.predecessor_digest && target == state.target {
                    // Retain the exact prepared successor. A restarted provider may only
                    // re-prepare these same bytes or prove the predecessor and abort it.
                } else if observed_prefix_digest == prepared.successor_digest
                    && target
                        == prepared
                            .successor_target
                            .clone()
                            .unwrap_or_else(|| state.target.clone())
                {
                    state.acknowledged_prefix_digest = prepared.successor_digest;
                    state.epoch = prepared.epoch;
                    if let Some(successor_target) = prepared.successor_target {
                        state.target = successor_target;
                    }
                    state.prepared = None;
                } else {
                    return Err(ProviderTestError::Invalid);
                }
            } else if target != state.target
                || observed_prefix_digest != state.acknowledged_prefix_digest
            {
                return Err(ProviderTestError::Invalid);
            }
            Ok(CheckpointReply::Current { epoch: state.epoch })
        }
        CheckpointRequest::Recover {
            fence_lineage_ref,
            target,
            observed_prefix_digest,
        } => {
            if fence_lineage_ref != state.fence_lineage_ref {
                return Err(ProviderTestError::Invalid);
            }
            if let Some(prepared) = state.prepared.clone() {
                if observed_prefix_digest == prepared.predecessor_digest && target == state.target {
                    state.prepared = None;
                } else if observed_prefix_digest == prepared.successor_digest
                    && target
                        == prepared
                            .successor_target
                            .clone()
                            .unwrap_or_else(|| state.target.clone())
                {
                    state.acknowledged_prefix_digest = prepared.successor_digest;
                    state.epoch = prepared.epoch;
                    if let Some(successor_target) = prepared.successor_target {
                        state.target = successor_target;
                    }
                    state.prepared = None;
                } else {
                    return Err(ProviderTestError::Invalid);
                }
            } else if target != state.target
                || observed_prefix_digest != state.acknowledged_prefix_digest
            {
                return Err(ProviderTestError::Invalid);
            }
            Ok(CheckpointReply::Current { epoch: state.epoch })
        }
        CheckpointRequest::Prepare {
            fence_lineage_ref,
            target,
            expected_epoch,
            prepared,
        } => {
            validate_checkpoint_digest(&prepared.predecessor_digest)?;
            validate_checkpoint_digest(&prepared.successor_digest)?;
            validate_checkpoint_digest(&prepared.operation_digest)?;
            if fence_lineage_ref != state.fence_lineage_ref
                || target != state.target
                || expected_epoch != state.epoch
                || prepared.epoch
                    != state
                        .epoch
                        .checked_add(1)
                        .ok_or(ProviderTestError::Invalid)?
                || prepared.predecessor_digest != state.acknowledged_prefix_digest
                || prepared.successor_digest == prepared.predecessor_digest
            {
                return Err(ProviderTestError::Invalid);
            }
            match &state.prepared {
                Some(existing) if existing == &prepared => {}
                Some(_) => return Err(ProviderTestError::Invalid),
                None => state.prepared = Some(prepared.clone()),
            }
            Ok(CheckpointReply::Current {
                epoch: prepared.epoch,
            })
        }
        CheckpointRequest::Acknowledge {
            fence_lineage_ref,
            target,
            prepared,
            observed_prefix_digest,
        } => {
            if fence_lineage_ref != state.fence_lineage_ref
                || target != state.target
                || state.prepared.as_ref() != Some(&prepared)
                || observed_prefix_digest != prepared.successor_digest
            {
                return Err(ProviderTestError::Invalid);
            }
            state.epoch = prepared.epoch;
            state.acknowledged_prefix_digest = prepared.successor_digest;
            if let Some(successor_target) = prepared.successor_target {
                state.target = successor_target;
            }
            state.prepared = None;
            Ok(CheckpointReply::Current { epoch: state.epoch })
        }
        CheckpointRequest::Abort {
            fence_lineage_ref,
            target,
            prepared,
            observed_prefix_digest,
        } => {
            if fence_lineage_ref != state.fence_lineage_ref
                || target != state.target
                || state.prepared.as_ref() != Some(&prepared)
                || observed_prefix_digest != prepared.predecessor_digest
            {
                return Err(ProviderTestError::Invalid);
            }
            state.prepared = None;
            Ok(CheckpointReply::Current { epoch: state.epoch })
        }
        CheckpointRequest::Shutdown => Err(ProviderTestError::Invalid),
    }
}

fn validate_checkpoint_digest(digest: &str) -> Result<(), ProviderTestError> {
    ContentDigest::from_str(digest)
        .map(|_| ())
        .map_err(|_| ProviderTestError::Invalid)
}

fn valid_checkpoint_authentication_token(token_hex: &str) -> bool {
    if token_hex.len() != 64 {
        return false;
    }
    let mut token = Zeroizing::new([0_u8; 32]);
    hex::decode_to_slice(token_hex, token.as_mut()).is_ok()
}

struct CheckpointClient {
    endpoint: PathBuf,
    authentication_token: Zeroizing<[u8; 32]>,
    fence_lineage_ref: EvmWalletReference,
    epoch: tokio::sync::Mutex<u64>,
}

impl CheckpointClient {
    fn new(
        endpoint: PathBuf,
        authentication_token_hex: String,
        fence_lineage_ref: EvmWalletReference,
    ) -> Result<Self, ProviderTestError> {
        let mut authentication_token = Zeroizing::new([0_u8; 32]);
        hex::decode_to_slice(&authentication_token_hex, authentication_token.as_mut())
            .map_err(|_| ProviderTestError::Invalid)?;
        Ok(Self {
            endpoint,
            authentication_token,
            fence_lineage_ref,
            epoch: tokio::sync::Mutex::new(0),
        })
    }

    async fn verify(
        &self,
        target: &CheckpointTarget,
        prefix: &WalletCheckpointPrefix,
    ) -> Result<(), ProviderTestError> {
        let observed_prefix_digest = prefix.digest()?;
        let reply = self
            .request(&CheckpointRequest::Verify {
                fence_lineage_ref: self.fence_lineage_ref.clone(),
                target: target.clone(),
                observed_prefix_digest,
            })
            .await?;
        let CheckpointReply::Current { epoch } = reply else {
            return Err(ProviderTestError::Invalid);
        };
        *self.epoch.lock().await = epoch;
        Ok(())
    }

    async fn recover(
        &self,
        target: &CheckpointTarget,
        prefix: &WalletCheckpointPrefix,
    ) -> Result<(), ProviderTestError> {
        let observed_prefix_digest = prefix.digest()?;
        let reply = self
            .request(&CheckpointRequest::Recover {
                fence_lineage_ref: self.fence_lineage_ref.clone(),
                target: target.clone(),
                observed_prefix_digest,
            })
            .await?;
        let CheckpointReply::Current { epoch } = reply else {
            return Err(ProviderTestError::Invalid);
        };
        *self.epoch.lock().await = epoch;
        Ok(())
    }

    async fn prepare(
        &self,
        target: &CheckpointTarget,
        prefix: WalletCheckpointPrefix,
        mutation: &ProviderMutation,
        successor_target: Option<CheckpointTarget>,
    ) -> Result<PreparedCheckpoint, ProviderTestError> {
        let predecessor_digest = prefix.digest()?;
        let successor_digest = prefix.exact_successor(mutation)?.digest()?;
        let operation_digest = mfm_journal::structured::domain_content_digest(
            "mfm.wallet-authority-provider.checkpoint-operation.v1",
            mutation,
        )
        .map_err(|_| ProviderTestError::Invalid)?
        .as_str()
        .to_owned();
        let epoch = *self.epoch.lock().await;
        let prepared = PreparedCheckpoint {
            epoch: epoch.checked_add(1).ok_or(ProviderTestError::Invalid)?,
            predecessor_digest,
            successor_digest,
            operation_digest,
            successor_target,
        };
        match self
            .request(&CheckpointRequest::Prepare {
                fence_lineage_ref: self.fence_lineage_ref.clone(),
                target: target.clone(),
                expected_epoch: epoch,
                prepared: prepared.clone(),
            })
            .await?
        {
            CheckpointReply::Current { epoch } if epoch == prepared.epoch => Ok(prepared),
            _ => Err(ProviderTestError::Invalid),
        }
    }

    async fn acknowledge(
        &self,
        target: &CheckpointTarget,
        prefix: &WalletCheckpointPrefix,
        prepared: PreparedCheckpoint,
    ) -> Result<(), ProviderTestError> {
        let observed_prefix_digest = prefix.digest()?;
        let expected_epoch = prepared.epoch;
        let reply = self
            .request(&CheckpointRequest::Acknowledge {
                fence_lineage_ref: self.fence_lineage_ref.clone(),
                target: target.clone(),
                prepared,
                observed_prefix_digest,
            })
            .await?;
        match reply {
            CheckpointReply::Current { epoch } if epoch == expected_epoch => {
                *self.epoch.lock().await = epoch;
                Ok(())
            }
            _ => Err(ProviderTestError::Invalid),
        }
    }

    async fn abort(
        &self,
        target: &CheckpointTarget,
        prefix: &WalletCheckpointPrefix,
        prepared: PreparedCheckpoint,
    ) -> Result<(), ProviderTestError> {
        let observed_prefix_digest = prefix.digest()?;
        let expected_epoch = *self.epoch.lock().await;
        let reply = self
            .request(&CheckpointRequest::Abort {
                fence_lineage_ref: self.fence_lineage_ref.clone(),
                target: target.clone(),
                prepared,
                observed_prefix_digest,
            })
            .await?;
        match reply {
            CheckpointReply::Current { epoch } if epoch == expected_epoch => Ok(()),
            _ => Err(ProviderTestError::Invalid),
        }
    }

    async fn request(
        &self,
        request: &CheckpointRequest,
    ) -> Result<CheckpointReply, ProviderTestError> {
        let stream =
            tokio::time::timeout(CHECKPOINT_IO_TIMEOUT, UnixStream::connect(&self.endpoint))
                .await
                .map_err(|_| ProviderTestError::Unavailable)?
                .map_err(|_| ProviderTestError::Unavailable)?;
        let (read, mut write) = tokio::io::split(stream);
        let envelope =
            authenticate_checkpoint_request(self.authentication_token.as_ref(), request.clone())?;
        let encoded = serde_json::to_vec(&envelope).map_err(|_| ProviderTestError::Invalid)?;
        tokio::time::timeout(CHECKPOINT_IO_TIMEOUT, async {
            write.write_all(&encoded).await?;
            write.write_all(b"\n").await?;
            write.flush().await
        })
        .await
        .map_err(|_| ProviderTestError::Unavailable)?
        .map_err(|_| ProviderTestError::Unavailable)?;
        let mut read = BufReader::new(read);
        let reply = tokio::time::timeout(
            CHECKPOINT_IO_TIMEOUT,
            read_bounded_frame(&mut read, MAX_MESSAGE_BYTES),
        )
        .await
        .map_err(|_| ProviderTestError::Unavailable)?
        .map_err(|_| ProviderTestError::Unavailable)?;
        serde_json::from_slice(&reply).map_err(|_| ProviderTestError::Invalid)
    }
}

/// Complete private startup configuration sent over the provider's stdin.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProcessConfig {
    socket_path: PathBuf,
    checkpoint_socket_path: PathBuf,
    checkpoint_authentication_token_hex: String,
    database_url: String,
    nonce_database_url: String,
    schema_name: String,
    provider_id: String,
    fence_lineage_ref: EvmWalletReference,
    chain_registry_lineage_ref: EvmWalletReference,
    current_chain_registry_head_ref: EvmWalletReference,
    registry_lineage_ref: EvmWalletReference,
    provider_fence_head_ref: EvmWalletReference,
    current_incarnation: WalletNonceStoreIncarnation,
    allowed_issuance_incarnations: Vec<WalletNonceStoreIncarnation>,
    current_public_lineage_head: HistoryObject,
    routing_catalog_history: Vec<EvmRoutingCatalogDescriptor>,
    #[serde(default)]
    rpc_inventory_targets: Vec<ProviderRpcInventoryTarget>,
    #[serde(default)]
    deployment_assembly_policies: Vec<ProviderDeploymentAssemblyPolicy>,
    #[serde(default)]
    hostile_deployment_assembly_lease_sequence: Vec<String>,
    allowed_activation_records: Vec<WalletNonceDomainActivationRecord>,
    allowed_promotions: Vec<AllowedPromotion>,
    supersession: Option<SupersessionConfig>,
    signing_seed_hex: String,
    startup_state: ProviderStartupState,
    promotion_crash_point: Option<ProviderPromotionCrashPoint>,
}

/// One private RPC-target proof key bound to an exact routing generation.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRpcInventoryTarget {
    route_generation_ref: ContentRef,
    target_identity_hex: String,
    proof_key_hex: String,
}

/// Exact signer, semantic-contract, and release closure admitted by the provider.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDeploymentAssemblyPolicy {
    semantic_signer_id: StableId,
    signer_generation_ref: ContentRef,
    signer_fence_ref: ContentRef,
    direct_sign_exclusion_ref: ContentRef,
    semantic_contract_refs: Vec<ContentRef>,
    release_history_digests: Vec<ContentDigest>,
}

impl ProviderDeploymentAssemblyPolicy {
    /// Constructs one exact provider-owned deployment policy.
    pub fn new(
        semantic_signer_id: StableId,
        signer_generation_ref: ContentRef,
        signer_fence_ref: ContentRef,
        direct_sign_exclusion_ref: ContentRef,
        semantic_contract_refs: Vec<ContentRef>,
        release_history_digests: Vec<ContentDigest>,
    ) -> Result<Self, ProviderTestError> {
        if semantic_contract_refs.len() != 6 || release_history_digests.len() != 6 {
            return Err(ProviderTestError::Invalid);
        }
        Ok(Self {
            semantic_signer_id,
            signer_generation_ref,
            signer_fence_ref,
            direct_sign_exclusion_ref,
            semantic_contract_refs,
            release_history_digests,
        })
    }
}

impl ProviderRpcInventoryTarget {
    /// Constructs one exact private target binding for provider qualification tests.
    pub fn new(
        route_generation_ref: ContentRef,
        target_identity: [u8; 32],
        proof_key: [u8; 32],
    ) -> Self {
        Self {
            route_generation_ref,
            target_identity_hex: hex::encode(target_identity),
            proof_key_hex: hex::encode(proof_key),
        }
    }
}

/// Durable external-control-plane phase used when restarting a qualification provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStartupState {
    /// The current incarnation accepts new affine leases.
    CurrentOpen,
    /// The old incarnation is fenced and its complete quiescent prefix is retained.
    FencedPromotionReady {
        /// Complete closed-prefix digest retained by the external control plane
        /// before the prior provider process stopped.
        captured_prefix_digest: String,
    },
    /// The registry CAS committed, but the promoted replacement remains closed.
    PromotedClosed,
    /// The registry CAS committed and the promoted replacement is open.
    PromotedOpen,
}

/// Process-crash boundary injected during one provider promotion confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderPromotionCrashPoint {
    /// Exit after observing the committed registry CAS and before opening the replacement.
    AfterCasBeforeOpen,
    /// Exit after opening the replacement and before acknowledging confirmation.
    AfterOpenBeforeConfirmation,
}

/// One provider-authorized successor plus its public post-CAS head.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedPromotion {
    /// Exact desired successor identity.
    pub successor: WalletNonceStoreSuccessor,
    /// Public lineage-head object for the successor.
    pub next_public_lineage_head: HistoryObject,
    /// Next provider fence head after promotion.
    pub next_provider_fence_head_ref: EvmWalletReference,
}

/// Public supersession evidence emitted after provider revocation.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupersessionConfig {
    /// Monotonic typed resource evidence.
    pub evidence: WalletNonceStoreLineageHead,
    /// Exact public object resolving the evidence.
    pub public_head: HistoryObject,
}

impl ProviderProcessConfig {
    /// Constructs the exact configuration for one separate provider process.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        socket_path: PathBuf,
        checkpoint_authority: &ProviderCheckpointAuthority,
        database_url: String,
        nonce_database_url: String,
        schema_name: String,
        provider_id: StableId,
        fence_lineage_ref: EvmWalletReference,
        chain_registry_lineage_ref: EvmWalletReference,
        current_chain_registry_head_ref: EvmWalletReference,
        registry_lineage_ref: EvmWalletReference,
        provider_fence_head_ref: EvmWalletReference,
        current_incarnation: WalletNonceStoreIncarnation,
        current_public_lineage_head: HistoryObject,
        routing_catalog_history: Vec<EvmRoutingCatalogDescriptor>,
        allowed_activation_records: Vec<WalletNonceDomainActivationRecord>,
        allowed_promotions: Vec<AllowedPromotion>,
        supersession: Option<SupersessionConfig>,
    ) -> Result<Self, ProviderTestError> {
        let signing_seed_hex = hex::encode(random_bytes::<32>()?.as_ref());
        let config = Self {
            socket_path,
            checkpoint_socket_path: checkpoint_authority.endpoint.clone(),
            checkpoint_authentication_token_hex: hex::encode(
                checkpoint_authority.authentication_token.as_ref(),
            ),
            database_url,
            nonce_database_url,
            schema_name,
            provider_id: provider_id.as_str().to_owned(),
            fence_lineage_ref,
            chain_registry_lineage_ref,
            current_chain_registry_head_ref,
            registry_lineage_ref,
            provider_fence_head_ref,
            allowed_issuance_incarnations: vec![current_incarnation.clone()],
            current_incarnation,
            current_public_lineage_head,
            routing_catalog_history,
            rpc_inventory_targets: Vec::new(),
            deployment_assembly_policies: Vec::new(),
            hostile_deployment_assembly_lease_sequence: Vec::new(),
            allowed_activation_records,
            allowed_promotions,
            supersession,
            signing_seed_hex,
            startup_state: ProviderStartupState::CurrentOpen,
            promotion_crash_point: None,
        };
        config.validate_contract()?;
        Ok(config)
    }

    /// Installs the complete private RPC target inventory for deployment assembly tests.
    pub fn with_rpc_inventory_targets(
        mut self,
        targets: Vec<ProviderRpcInventoryTarget>,
    ) -> Result<Self, ProviderTestError> {
        self.rpc_inventory_targets = targets;
        self.validate_contract()?;
        Ok(self)
    }

    /// Installs the complete provider-owned deployment assembly allowlist.
    pub fn with_deployment_assembly_policies(
        mut self,
        policies: Vec<ProviderDeploymentAssemblyPolicy>,
    ) -> Result<Self, ProviderTestError> {
        self.deployment_assembly_policies = policies;
        self.validate_contract()?;
        Ok(self)
    }

    /// Forces the next bounded sequence of deployment Begin markers in hostile protocol tests.
    pub fn with_hostile_deployment_assembly_lease_sequence(
        mut self,
        leases: Vec<[u8; 32]>,
    ) -> Result<Self, ProviderTestError> {
        if leases.is_empty() || leases.len() > 16 {
            return Err(ProviderTestError::Invalid);
        }
        self.hostile_deployment_assembly_lease_sequence =
            leases.into_iter().map(hex::encode).collect();
        self.validate_contract()?;
        Ok(self)
    }

    /// Returns the provider's public socket endpoint.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Allows additional exact initial incarnations for activation-race qualification.
    pub fn with_allowed_issuance_incarnations(
        mut self,
        incarnations: Vec<WalletNonceStoreIncarnation>,
    ) -> Result<Self, ProviderTestError> {
        if incarnations.is_empty()
            || incarnations
                .iter()
                .any(|incarnation| incarnation.validate().is_err())
        {
            return Err(ProviderTestError::Invalid);
        }
        self.allowed_issuance_incarnations = incarnations;
        Ok(self)
    }

    /// Replaces the complete append-only routing history for an ordinary provider restart.
    ///
    /// The final catalog must carry `current_chain_registry_head_ref` and retain every permanent
    /// binding from every earlier catalog exactly.
    pub fn with_routing_catalog_history(
        mut self,
        current_chain_registry_head_ref: EvmWalletReference,
        routing_catalog_history: Vec<EvmRoutingCatalogDescriptor>,
    ) -> Result<Self, ProviderTestError> {
        self.current_chain_registry_head_ref = current_chain_registry_head_ref;
        self.routing_catalog_history = routing_catalog_history;
        self.validate_contract()?;
        Ok(self)
    }

    /// Restarts the provider at one durable promotion phase.
    pub fn with_startup_state(
        mut self,
        startup_state: ProviderStartupState,
    ) -> Result<Self, ProviderTestError> {
        if !matches!(&startup_state, ProviderStartupState::CurrentOpen)
            && self.allowed_promotions.len() != 1
        {
            return Err(ProviderTestError::Invalid);
        }
        if matches!(
            &startup_state,
            ProviderStartupState::FencedPromotionReady {
                captured_prefix_digest
            } if ContentDigest::from_str(captured_prefix_digest).is_err()
        ) {
            return Err(ProviderTestError::Invalid);
        }
        self.startup_state = startup_state;
        Ok(self)
    }

    /// Injects one abrupt process exit while confirming the configured promotion.
    pub fn with_promotion_crash_point(
        mut self,
        crash_point: ProviderPromotionCrashPoint,
    ) -> Result<Self, ProviderTestError> {
        if self.allowed_promotions.len() != 1 {
            return Err(ProviderTestError::Invalid);
        }
        self.promotion_crash_point = Some(crash_point);
        Ok(self)
    }

    fn validate_contract(&self) -> Result<(), ProviderTestError> {
        if self.socket_path.as_os_str().is_empty()
            || self.checkpoint_socket_path.as_os_str().is_empty()
            || self.checkpoint_socket_path == self.socket_path
            || !valid_checkpoint_authentication_token(&self.checkpoint_authentication_token_hex)
            || std::fs::metadata(&self.checkpoint_socket_path)
                .map(|metadata| metadata.permissions().mode() & 0o077 != 0)
                .unwrap_or(true)
            || self.schema_name.is_empty()
            || self.database_url.is_empty()
            || self.nonce_database_url.is_empty()
            || self.routing_catalog_history.is_empty()
            || self.allowed_activation_records.is_empty()
            || StableId::new(&self.provider_id).is_err()
        {
            return Err(ProviderTestError::Invalid);
        }
        self.current_incarnation
            .validate()
            .map_err(|_| ProviderTestError::Invalid)?;
        self.current_public_lineage_head
            .validate()
            .map_err(|_| ProviderTestError::Invalid)?;
        if self
            .allowed_issuance_incarnations
            .iter()
            .any(|incarnation| incarnation.validate().is_err())
        {
            return Err(ProviderTestError::Invalid);
        }
        EvmRoutingCatalogDescriptor::validate_append_only_history(
            &self.chain_registry_lineage_ref,
            &self.current_chain_registry_head_ref,
            &self.routing_catalog_history,
        )
        .map_err(|_| ProviderTestError::Invalid)?;
        let current_catalog = self
            .routing_catalog_history
            .last()
            .ok_or(ProviderTestError::Invalid)?;
        let mut inventory_routes = BTreeMap::new();
        for target in &self.rpc_inventory_targets {
            let target_identity =
                hex::decode(&target.target_identity_hex).map_err(|_| ProviderTestError::Invalid)?;
            let proof_key =
                hex::decode(&target.proof_key_hex).map_err(|_| ProviderTestError::Invalid)?;
            if target_identity.len() != 32
                || proof_key.len() != 32
                || current_catalog.generations().iter().all(|generation| {
                    generation
                        .generation_ref()
                        .and_then(|reference| {
                            reference.to_content_ref().map_err(|_| {
                                mfm_evm::WalletAuthorityContractError::Invalid("route")
                            })
                        })
                        .ok()
                        .as_ref()
                        != Some(&target.route_generation_ref)
                })
                || inventory_routes
                    .insert(target.route_generation_ref.clone(), ())
                    .is_some()
            {
                return Err(ProviderTestError::Invalid);
            }
        }
        let mut policy_signers = BTreeMap::new();
        for policy in &self.deployment_assembly_policies {
            if policy.semantic_contract_refs.len() != 6
                || policy.release_history_digests.len() != 6
                || policy_signers
                    .insert(policy.semantic_signer_id.clone(), ())
                    .is_some()
            {
                return Err(ProviderTestError::Invalid);
            }
        }
        if self.hostile_deployment_assembly_lease_sequence.len() > 16
            || self
                .hostile_deployment_assembly_lease_sequence
                .iter()
                .any(|lease| lease.len() != 64 || decode_canonical_bounded_hex(lease, 32).is_err())
        {
            return Err(ProviderTestError::Invalid);
        }
        for record in &self.allowed_activation_records {
            record.validate().map_err(|_| ProviderTestError::Invalid)?;
            if !catalog_admits_activation(current_catalog, record) {
                return Err(ProviderTestError::Invalid);
            }
        }
        validate_authority_reference_roles(self)
    }
}

fn validate_authority_reference_roles(
    config: &ProviderProcessConfig,
) -> Result<(), ProviderTestError> {
    let mut roles = BTreeMap::new();
    retain_authority_reference_role(
        &mut roles,
        &config.fence_lineage_ref,
        "provider_fence_lineage",
    )?;
    retain_authority_reference_role(
        &mut roles,
        &config.registry_lineage_ref,
        "activation_registry_lineage",
    )?;
    retain_authority_reference_role(
        &mut roles,
        &config.provider_fence_head_ref,
        "provider_fence_head",
    )?;
    for promotion in &config.allowed_promotions {
        retain_authority_reference_role(
            &mut roles,
            &promotion.next_provider_fence_head_ref,
            "provider_fence_head",
        )?;
    }
    retain_authority_reference_role(
        &mut roles,
        &config.chain_registry_lineage_ref,
        "chain_registry_lineage",
    )?;
    retain_authority_reference_role(
        &mut roles,
        &config.current_chain_registry_head_ref,
        "chain_registry_head",
    )?;
    for catalog in &config.routing_catalog_history {
        retain_authority_reference_role(
            &mut roles,
            catalog.chain_registry_head_ref(),
            "chain_registry_head",
        )?;
        for attestation in catalog.chain_instances() {
            retain_authority_reference_role(
                &mut roles,
                attestation.declaration().chain_registry_lineage_ref(),
                "chain_registry_lineage",
            )?;
            retain_authority_reference_role(
                &mut roles,
                attestation.registry_head_ref_at_issuance(),
                "chain_registry_head",
            )?;
            retain_authority_reference_role(
                &mut roles,
                attestation.registry_issuance_ref(),
                "chain_registry_issuance",
            )?;
        }
        for generation in catalog.generations() {
            retain_authority_reference_role(
                &mut roles,
                generation.route_membership_issuance_ref(),
                "route_membership_issuance",
            )?;
        }
    }
    Ok(())
}

fn retain_authority_reference_role(
    retained: &mut BTreeMap<EvmWalletReference, &'static str>,
    reference: &EvmWalletReference,
    role: &'static str,
) -> Result<(), ProviderTestError> {
    reference
        .to_content_ref()
        .map_err(|_| ProviderTestError::Invalid)?;
    if retained
        .get(reference)
        .is_some_and(|retained_role| *retained_role != role)
    {
        return Err(ProviderTestError::Invalid);
    }
    retained.insert(reference.clone(), role);
    Ok(())
}

fn catalog_admits_activation(
    catalog: &EvmRoutingCatalogDescriptor,
    record: &WalletNonceDomainActivationRecord,
) -> bool {
    let Ok(chain_instance) = record.chain_instance_attestation.binding() else {
        return false;
    };
    catalog
        .chain_instances()
        .contains(&record.chain_instance_attestation)
        && catalog
            .resolve_generation(&record.initial_route_generation_ref)
            .is_some_and(|generation| {
                generation.chain_instance() == &chain_instance
                    && generation.route_membership_issuance_ref()
                        == &record.initial_route_membership_issuance_ref
            })
}

/// Parent-side handle for one live separate provider process.
pub struct ProviderProcess {
    child: Child,
    control: ChildStdin,
    output: SyncBufReader<ChildStdout>,
    endpoint: PathBuf,
    provider_public_key_hex: String,
}

impl ProviderProcess {
    /// Spawns the exact helper executable and transfers private configuration
    /// through stdin rather than argv or environment variables.
    pub fn spawn(
        executable: impl AsRef<Path>,
        config: ProviderProcessConfig,
    ) -> Result<Self, ProviderTestError> {
        let endpoint = config.socket_path.clone();
        if endpoint.exists() {
            return Err(ProviderTestError::Invalid);
        }
        let mut child = Command::new(executable.as_ref())
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ProviderTestError::Unavailable)?;
        let startup = (|| {
            let mut control = child.stdin.take().ok_or(ProviderTestError::Unavailable)?;
            serde_json::to_writer(&mut control, &config).map_err(|_| ProviderTestError::Invalid)?;
            control
                .write_all(b"\n")
                .map_err(|_| ProviderTestError::Unavailable)?;
            control
                .flush()
                .map_err(|_| ProviderTestError::Unavailable)?;
            let stdout = child.stdout.take().ok_or(ProviderTestError::Unavailable)?;
            let mut output = SyncBufReader::new(stdout);
            let line = read_bounded_sync_frame(&mut output, MAX_MESSAGE_BYTES)
                .map_err(|_| ProviderTestError::Unavailable)?;
            let ready: ProviderReady =
                serde_json::from_slice(&line).map_err(|_| ProviderTestError::Unavailable)?;
            if !ready.ready || ready.public_key_hex.len() != 64 {
                return Err(ProviderTestError::Unavailable);
            }
            Ok((control, output, ready.public_key_hex))
        })();
        match startup {
            Ok((control, output, provider_public_key_hex)) => Ok(Self {
                child,
                control,
                output,
                endpoint,
                provider_public_key_hex,
            }),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                match std::fs::remove_file(&endpoint) {
                    Ok(()) => {}
                    Err(remove_error) if remove_error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(ProviderTestError::Unavailable),
                }
                Err(error)
            }
        }
    }

    /// Returns the live provider endpoint.
    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }

    /// Returns the provider's public authentication key.
    pub fn public_key_hex(&self) -> &str {
        &self.provider_public_key_hex
    }

    /// Stops new leases and waits for existing affine leases to drain.
    pub fn revoke(&mut self) -> Result<(), ProviderTestError> {
        self.control_command("revoke")
    }

    /// Authorizes the configured successor after revocation and returns the
    /// closed-prefix digest that the external control plane must retain.
    pub fn authorize_promotion(&mut self) -> Result<String, ProviderTestError> {
        self.control
            .write_all(b"authorize_promotion\n")
            .and_then(|()| self.control.flush())
            .map_err(|_| ProviderTestError::Unavailable)?;
        let line = self.read_control_response()?;
        let digest = std::str::from_utf8(&line).map_err(|_| ProviderTestError::Unavailable)?;
        if ContentDigest::from_str(digest).is_err() {
            Err(ProviderTestError::Unavailable)
        } else {
            Ok(digest.to_owned())
        }
    }

    /// Simulates loss of the captured closed-prefix hydration proof.
    pub fn forget_hydrated_prefix(&mut self) -> Result<(), ProviderTestError> {
        self.control_command("forget_hydrated_prefix")
    }

    /// Returns the provider's live affine-lease count for drain qualification.
    pub fn active_leases(&mut self) -> Result<usize, ProviderTestError> {
        self.control
            .write_all(b"active_leases\n")
            .and_then(|()| self.control.flush())
            .map_err(|_| ProviderTestError::Unavailable)?;
        let line = self.read_control_response()?;
        std::str::from_utf8(&line)
            .map_err(|_| ProviderTestError::Unavailable)?
            .parse()
            .map_err(|_| ProviderTestError::Unavailable)
    }

    /// Returns how many armed affine leases passed their signed entry permit and remain held.
    pub fn entered_affine_lease_holds(&mut self) -> Result<usize, ProviderTestError> {
        self.control
            .write_all(b"entered_affine_lease_holds\n")
            .and_then(|()| self.control.flush())
            .map_err(|_| ProviderTestError::Unavailable)?;
        let line = self.read_control_response()?;
        std::str::from_utf8(&line)
            .map_err(|_| ProviderTestError::Unavailable)?
            .parse()
            .map_err(|_| ProviderTestError::Unavailable)
    }

    /// Arms a bounded pause after each of the next `count` affine leases is acquired.
    pub fn hold_next_affine_leases(&mut self, count: u64) -> Result<(), ProviderTestError> {
        if !(1..=16).contains(&count) {
            return Err(ProviderTestError::Invalid);
        }
        self.control_command(&format!("hold_next_affine_leases {count}"))
    }

    /// Returns the number of explicit chain-registry catalog qualifications.
    pub fn routing_qualification_calls(&mut self) -> Result<u64, ProviderTestError> {
        self.control
            .write_all(b"routing_qualification_calls\n")
            .and_then(|()| self.control.flush())
            .map_err(|_| ProviderTestError::Unavailable)?;
        let line = self.read_control_response()?;
        std::str::from_utf8(&line)
            .map_err(|_| ProviderTestError::Unavailable)?
            .parse()
            .map_err(|_| ProviderTestError::Unavailable)
    }

    /// Returns the number of deployment Finish requests that released an authorization.
    pub fn successful_deployment_finishes(&mut self) -> Result<u64, ProviderTestError> {
        self.control
            .write_all(b"successful_deployment_finishes\n")
            .and_then(|()| self.control.flush())
            .map_err(|_| ProviderTestError::Unavailable)?;
        let line = self.read_control_response()?;
        std::str::from_utf8(&line)
            .map_err(|_| ProviderTestError::Unavailable)?
            .parse()
            .map_err(|_| ProviderTestError::Unavailable)
    }

    /// Abruptly terminates the provider without its graceful shutdown path.
    pub fn crash(&mut self) -> Result<(), ProviderTestError> {
        self.child
            .kill()
            .and_then(|()| self.child.wait().map(|_| ()))
            .map_err(|_| ProviderTestError::Unavailable)?;
        std::fs::remove_file(&self.endpoint).map_err(|_| ProviderTestError::Unavailable)
    }

    fn control_command(&mut self, command: &str) -> Result<(), ProviderTestError> {
        self.control
            .write_all(command.as_bytes())
            .and_then(|()| self.control.write_all(b"\n"))
            .and_then(|()| self.control.flush())
            .map_err(|_| ProviderTestError::Unavailable)?;
        let line = self.read_control_response()?;
        if line.as_slice() == b"ok" {
            Ok(())
        } else {
            Err(ProviderTestError::Unavailable)
        }
    }

    fn read_control_response(&mut self) -> Result<Zeroizing<Vec<u8>>, ProviderTestError> {
        read_bounded_sync_frame(&mut self.output, MAX_MESSAGE_BYTES)
            .map_err(|_| ProviderTestError::Unavailable)
    }
}

impl Drop for ProviderProcess {
    fn drop(&mut self) {
        let _ = self.control.write_all(b"shutdown\n");
        let _ = self.control.flush();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.endpoint);
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderReady {
    ready: bool,
    public_key_hex: String,
}

/// Runs the test provider using one private stdin configuration and a public
/// readiness record on stdout.
pub async fn run_provider_from_stdio() -> Result<(), ProviderTestError> {
    let mut stdin = BufReader::new(tokio::io::stdin());
    let config_line = read_bounded_frame(&mut stdin, MAX_MESSAGE_BYTES)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?;
    let mut config: ProviderProcessConfig =
        serde_json::from_slice(&config_line).map_err(|_| ProviderTestError::Invalid)?;
    config.validate_contract()?;
    if config.socket_path.exists() {
        return Err(ProviderTestError::Invalid);
    }

    let mut signing_seed = Zeroizing::new([0_u8; 32]);
    hex::decode_to_slice(&config.signing_seed_hex, signing_seed.as_mut())
        .map_err(|_| ProviderTestError::Invalid)?;
    config.signing_seed_hex.zeroize();
    let key_pair = Arc::new(
        Ed25519KeyPair::from_seed_unchecked(signing_seed.as_ref())
            .map_err(|_| ProviderTestError::Unavailable)?,
    );
    let public_key_hex = hex::encode(key_pair.public_key().as_ref());
    let listener =
        UnixListener::bind(&config.socket_path).map_err(|_| ProviderTestError::Unavailable)?;
    let pool = PgPool::connect(&config.database_url)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?;
    let nonce_pool = PgPool::connect(&config.nonce_database_url)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?;
    let database_oid = sqlx::query_scalar::<_, i64>(
        "SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname = current_database()",
    )
    .fetch_one(&pool)
    .await
    .map_err(|_| ProviderTestError::Unavailable)?;
    let database_oid = u32::try_from(database_oid).map_err(|_| ProviderTestError::Invalid)?;

    if current_database_oid(&nonce_pool).await? != database_oid {
        return Err(ProviderTestError::Invalid);
    }
    let state = ProviderState::new(config, pool, nonce_pool, database_oid)?;
    state.verify_checkpoint().await?;
    state.hydrate_issued_activations().await?;
    let state = Arc::new(state);
    let ready = serde_json::to_string(&ProviderReady {
        ready: true,
        public_key_hex,
    })
    .map_err(|_| ProviderTestError::Invalid)?;
    let mut stdout = tokio::io::stdout();
    stdout
        .write_all(ready.as_bytes())
        .await
        .map_err(|_| ProviderTestError::Unavailable)?;
    stdout
        .write_all(b"\n")
        .await
        .map_err(|_| ProviderTestError::Unavailable)?;
    stdout
        .flush()
        .await
        .map_err(|_| ProviderTestError::Unavailable)?;

    let shutdown = Arc::new(Notify::new());
    let control_state = Arc::clone(&state);
    let control_shutdown = Arc::clone(&shutdown);
    let control = tokio::spawn(async move {
        loop {
            let line = match read_bounded_frame(&mut stdin, MAX_MESSAGE_BYTES).await {
                Ok(line) => line,
                Err(_) => {
                    control_shutdown.notify_waiters();
                    break;
                }
            };
            let Ok(command) = std::str::from_utf8(&line) else {
                control_shutdown.notify_waiters();
                break;
            };
            if command == "active_leases" {
                let response = control_state
                    .active_leases()
                    .map(|count| format!("{count}\n"))
                    .unwrap_or_else(|_| "error\n".to_owned());
                let _ = stdout.write_all(response.as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
            if command == "entered_affine_lease_holds" {
                let response = format!(
                    "{}\n",
                    control_state
                        .entered_affine_lease_holds
                        .load(Ordering::Acquire)
                );
                let _ = stdout.write_all(response.as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
            if command == "routing_qualification_calls" {
                let response = format!(
                    "{}\n",
                    control_state
                        .routing_qualification_calls
                        .load(Ordering::Acquire)
                );
                let _ = stdout.write_all(response.as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
            if command == "successful_deployment_finishes" {
                let response = format!("{}\n", control_state.successful_deployment_finishes());
                let _ = stdout.write_all(response.as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
            if let Some(count) = command.strip_prefix("hold_next_affine_leases ") {
                let result = count
                    .parse()
                    .map_err(|_| ProviderTestError::Invalid)
                    .and_then(|count| control_state.arm_affine_lease_holds(count));
                let response: &[u8] = if result.is_ok() { b"ok\n" } else { b"error\n" };
                let _ = stdout.write_all(response).await;
                let _ = stdout.flush().await;
                continue;
            }
            if command == "authorize_promotion" {
                let response = control_state
                    .authorize_promotion()
                    .await
                    .map(|digest| format!("{digest}\n"))
                    .unwrap_or_else(|_| "error\n".to_owned());
                let _ = stdout.write_all(response.as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
            let result = match command {
                "revoke" => control_state.revoke().await,
                "forget_hydrated_prefix" => control_state.forget_hydrated_prefix(),
                "shutdown" => {
                    control_shutdown.notify_waiters();
                    Ok(())
                }
                _ => Err(ProviderTestError::Invalid),
            };
            let response: &[u8] = if result.is_ok() { b"ok\n" } else { b"error\n" };
            let _ = stdout.write_all(response).await;
            let _ = stdout.flush().await;
            if command == "shutdown" {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|_| ProviderTestError::Unavailable)?;
                let connection_state = Arc::clone(&state);
                let connection_key = Arc::clone(&key_pair);
                tokio::spawn(async move {
                    let _ = serve_connection(stream, connection_state, connection_key).await;
                });
            }
            () = shutdown.notified() => break,
        }
    }
    control.abort();
    pool_close(&state).await;
    std::fs::remove_file(&state.socket_path).map_err(|_| ProviderTestError::Unavailable)?;
    Ok(())
}

async fn pool_close(state: &ProviderState) {
    state.pool.close().await;
    state.nonce_pool.close().await;
}

struct ProviderState {
    socket_path: PathBuf,
    pool: PgPool,
    nonce_pool: PgPool,
    database_oid: u32,
    checkpoint: CheckpointClient,
    checkpoint_observation: tokio::sync::Mutex<()>,
    provider_id: String,
    fence_lineage_ref: EvmWalletReference,
    chain_registry_lineage_ref: EvmWalletReference,
    current_chain_registry_head_ref: EvmWalletReference,
    registry_lineage_ref: EvmWalletReference,
    schema_name: String,
    current_routing_catalog: EvmRoutingCatalogDescriptor,
    rpc_inventory_targets: BTreeMap<ContentRef, RpcInventoryTarget>,
    deployment_assembly_policies: BTreeMap<StableId, ProviderDeploymentAssemblyPolicy>,
    allowed_activation_records: BTreeMap<String, WalletNonceDomainActivationRecord>,
    allowed_issuance_incarnations: BTreeMap<String, WalletNonceStoreIncarnation>,
    allowed_promotions: BTreeMap<String, AllowedPromotion>,
    supersession: Option<SupersessionConfig>,
    promotion_crash_point: Option<ProviderPromotionCrashPoint>,
    initial_provider_fence_head_ref: EvmWalletReference,
    routing_qualification_calls: AtomicU64,
    successful_deployment_finishes: AtomicU64,
    affine_lease_holds_remaining: AtomicU64,
    entered_affine_lease_holds: AtomicU64,
    mutable: Mutex<MutableProviderState>,
    leases_changed: Notify,
}

struct MutableProviderState {
    accepting: bool,
    promotion_authorized: bool,
    active_leases: usize,
    captured_prefix_digest: Option<String>,
    provider_fence_head_ref: EvmWalletReference,
    current_incarnation: WalletNonceStoreIncarnation,
    current_public_lineage_head: HistoryObject,
    issued: BTreeMap<String, WalletNonceDomainActivationAttestation>,
    issuance_provider_heads: BTreeMap<String, EvmWalletReference>,
    confirmed_promotions: BTreeMap<String, EvmWalletReference>,
    deployment_assemblies: DeploymentAssemblyLeaseTable<ProviderDeploymentAssembly>,
    hostile_deployment_assembly_lease_sequence: VecDeque<String>,
}

struct RpcInventoryTarget {
    target_identity: [u8; 32],
    proof_key: Zeroizing<[u8; 32]>,
}

struct ProviderDeploymentAssembly {
    descriptor: EvmRoutingCatalogDescriptor,
    provider_fence_head_ref: EvmWalletReference,
    binding: DeploymentAssemblyBindingWire,
    checkpoint: [u8; 32],
    finish_authorization: Zeroizing<Vec<u8>>,
    challenges: Vec<DeploymentAssemblyRouteChallengeWire>,
}

struct DeploymentAssemblyLease<T> {
    value: T,
    expires_at: Instant,
}

struct DeploymentAssemblyLeaseTable<T> {
    pending: BTreeMap<String, DeploymentAssemblyLease<T>>,
}

impl<T> Default for DeploymentAssemblyLeaseTable<T> {
    fn default() -> Self {
        Self {
            pending: BTreeMap::new(),
        }
    }
}

impl<T> DeploymentAssemblyLeaseTable<T> {
    fn prune(&mut self, now: Instant) {
        self.pending
            .retain(|_, lease| deployment_lease_is_current(lease.expires_at, now));
    }

    fn len(&self) -> usize {
        self.pending.len()
    }

    fn insert(&mut self, lease_id: String, value: T, expires_at: Instant) -> bool {
        match self.pending.entry(lease_id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(DeploymentAssemblyLease { value, expires_at });
                true
            }
            std::collections::btree_map::Entry::Occupied(_) => false,
        }
    }

    fn get(&self, lease_id: &str) -> Option<&T> {
        self.pending.get(lease_id).map(|lease| &lease.value)
    }

    fn remove(&mut self, lease_id: &str) -> Option<T> {
        self.pending.remove(lease_id).map(|lease| lease.value)
    }

    fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    fn clear(&mut self) {
        self.pending.clear();
    }
}

impl ProviderState {
    fn new(
        config: ProviderProcessConfig,
        pool: PgPool,
        nonce_pool: PgPool,
        database_oid: u32,
    ) -> Result<Self, ProviderTestError> {
        config.validate_contract()?;
        let checkpoint = CheckpointClient::new(
            config.checkpoint_socket_path.clone(),
            config.checkpoint_authentication_token_hex.clone(),
            config.fence_lineage_ref.clone(),
        )?;
        let current_routing_catalog = config
            .routing_catalog_history
            .last()
            .cloned()
            .ok_or(ProviderTestError::Invalid)?;
        let rpc_inventory_targets = config
            .rpc_inventory_targets
            .into_iter()
            .map(|target| {
                let target_identity: [u8; 32] = hex::decode(target.target_identity_hex)
                    .map_err(|_| ProviderTestError::Invalid)?
                    .try_into()
                    .map_err(|_| ProviderTestError::Invalid)?;
                let proof_key: [u8; 32] = hex::decode(target.proof_key_hex)
                    .map_err(|_| ProviderTestError::Invalid)?
                    .try_into()
                    .map_err(|_| ProviderTestError::Invalid)?;
                Ok((
                    target.route_generation_ref,
                    RpcInventoryTarget {
                        target_identity,
                        proof_key: Zeroizing::new(proof_key),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, ProviderTestError>>()?;
        let deployment_assembly_policies = config
            .deployment_assembly_policies
            .into_iter()
            .map(|policy| (policy.semantic_signer_id.clone(), policy))
            .collect::<BTreeMap<_, _>>();
        let allowed_activation_records = config
            .allowed_activation_records
            .into_iter()
            .map(|record| {
                let reference =
                    canonical_wallet_reference(&record).map_err(|_| ProviderTestError::Invalid)?;
                Ok((reference.content_digest().to_owned(), record))
            })
            .collect::<Result<BTreeMap<_, _>, ProviderTestError>>()?;
        let allowed_issuance_incarnations = config
            .allowed_issuance_incarnations
            .into_iter()
            .map(|incarnation| {
                let reference = canonical_wallet_reference(&incarnation)
                    .map_err(|_| ProviderTestError::Invalid)?;
                Ok((reference.content_digest().to_owned(), incarnation))
            })
            .collect::<Result<BTreeMap<_, _>, ProviderTestError>>()?;
        let mut allowed_promotions = BTreeMap::new();
        for promotion in config.allowed_promotions {
            let reference = canonical_wallet_reference(&promotion.successor)
                .map_err(|_| ProviderTestError::Invalid)?;
            allowed_promotions.insert(reference.content_digest().to_owned(), promotion);
        }
        let initial_provider_fence_head_ref = config.provider_fence_head_ref.clone();
        let mut mutable = MutableProviderState {
            accepting: true,
            promotion_authorized: false,
            active_leases: 0,
            captured_prefix_digest: None,
            provider_fence_head_ref: config.provider_fence_head_ref,
            current_incarnation: config.current_incarnation,
            current_public_lineage_head: config.current_public_lineage_head,
            issued: BTreeMap::new(),
            issuance_provider_heads: BTreeMap::new(),
            confirmed_promotions: BTreeMap::new(),
            deployment_assemblies: DeploymentAssemblyLeaseTable::default(),
            hostile_deployment_assembly_lease_sequence: config
                .hostile_deployment_assembly_lease_sequence
                .into(),
        };
        match config.startup_state {
            ProviderStartupState::CurrentOpen => {}
            ProviderStartupState::FencedPromotionReady {
                captured_prefix_digest,
            } => {
                ContentDigest::from_str(&captured_prefix_digest)
                    .map_err(|_| ProviderTestError::Invalid)?;
                mutable.accepting = false;
                mutable.promotion_authorized = true;
                mutable.captured_prefix_digest = Some(captured_prefix_digest);
            }
            startup_state @ (ProviderStartupState::PromotedClosed
            | ProviderStartupState::PromotedOpen) => {
                let (successor_digest, allowed) = allowed_promotions
                    .iter()
                    .next()
                    .ok_or(ProviderTestError::Invalid)?;
                mutable.accepting = matches!(startup_state, ProviderStartupState::PromotedOpen);
                mutable.provider_fence_head_ref = allowed.next_provider_fence_head_ref.clone();
                mutable.current_incarnation = allowed.successor.next_incarnation.clone();
                mutable.current_public_lineage_head = allowed.next_public_lineage_head.clone();
                mutable.confirmed_promotions.insert(
                    successor_digest.clone(),
                    initial_provider_fence_head_ref.clone(),
                );
            }
        }
        Ok(Self {
            socket_path: config.socket_path,
            pool,
            nonce_pool,
            database_oid,
            checkpoint,
            checkpoint_observation: tokio::sync::Mutex::new(()),
            provider_id: config.provider_id,
            fence_lineage_ref: config.fence_lineage_ref,
            chain_registry_lineage_ref: config.chain_registry_lineage_ref,
            current_chain_registry_head_ref: config.current_chain_registry_head_ref,
            registry_lineage_ref: config.registry_lineage_ref,
            schema_name: config.schema_name,
            current_routing_catalog,
            rpc_inventory_targets,
            deployment_assembly_policies,
            allowed_activation_records,
            allowed_issuance_incarnations,
            allowed_promotions,
            supersession: config.supersession,
            promotion_crash_point: config.promotion_crash_point,
            initial_provider_fence_head_ref: initial_provider_fence_head_ref.clone(),
            routing_qualification_calls: AtomicU64::new(0),
            successful_deployment_finishes: AtomicU64::new(0),
            affine_lease_holds_remaining: AtomicU64::new(0),
            entered_affine_lease_holds: AtomicU64::new(0),
            mutable: Mutex::new(mutable),
            leases_changed: Notify::new(),
        })
    }

    async fn hydrate_issued_activations(&self) -> Result<(), ProviderTestError> {
        let sql = format!(
            "SELECT wallet_nonce_domain_id, activation_record_ref, \
                    wallet_nonce_store_lineage_id, observed_store_incarnation_ref, \
                    activation_record_json, registry_issuance_ref, activation_attestation_json \
               FROM {}.wallet_domain_activations \
             ORDER BY wallet_nonce_domain_id",
            quoted_identifier(&self.schema_name)?
        );
        let retained = sqlx::query(AssertSqlSafe(sql))
            .fetch_all(&self.pool)
            .await
            .map_err(|_| ProviderTestError::Unavailable)?;
        let mut issued = BTreeMap::new();
        let mut provider_heads = BTreeMap::new();
        for row in retained {
            let attestation = decode_retained_activation(&row, None)?;
            let record_ref = canonical_wallet_reference(&attestation.current_schema_record)
                .map_err(|_| ProviderTestError::Invalid)?;
            let expected_issuance_ref =
                provider_issuance_reference(&self.provider_id, &record_ref)?;
            let initial_incarnation_allowed =
                self.allowed_issuance_incarnations
                    .iter()
                    .any(|(digest, incarnation)| {
                        digest == attestation.initial_store_incarnation_ref.content_digest()
                            && canonical_wallet_reference(incarnation).ok().as_ref()
                                == Some(&attestation.initial_store_incarnation_ref)
                    });
            if self
                .allowed_activation_records
                .get(record_ref.content_digest())
                != Some(&attestation.current_schema_record)
                || attestation.registry_issuance_ref != expected_issuance_ref
                || attestation.activation_registry_lineage_ref != self.registry_lineage_ref
                || !initial_incarnation_allowed
            {
                return Err(ProviderTestError::Invalid);
            }
            let domain = attestation
                .current_schema_record
                .wallet_nonce_domain
                .as_str()
                .to_owned();
            if issued.insert(domain.clone(), attestation).is_some() {
                return Err(ProviderTestError::Invalid);
            }
            provider_heads.insert(domain, self.initial_provider_fence_head_ref.clone());
        }
        let mut mutable = self
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        mutable.issued = issued;
        mutable.issuance_provider_heads = provider_heads;
        Ok(())
    }

    fn checkpoint_target(&self) -> Result<CheckpointTarget, ProviderTestError> {
        let retained = self
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        Ok(CheckpointTarget {
            database_oid: self.database_oid,
            schema_name: self.schema_name.clone(),
            store_incarnation: retained.current_incarnation.clone(),
            current_public_lineage_head: retained.current_public_lineage_head.clone(),
            provider_fence_head_ref: retained.provider_fence_head_ref.clone(),
        })
    }

    async fn verify_checkpoint(&self) -> Result<(), ProviderTestError> {
        let _observation = self.checkpoint_observation.lock().await;
        let target = self.checkpoint_target()?;
        let prefix =
            wallet_checkpoint_prefix(&self.pool, &self.nonce_pool, &self.schema_name).await?;
        self.checkpoint.verify(&target, &prefix).await
    }

    async fn recover_checkpoint_if_idle(&self) -> Result<(), ProviderTestError> {
        let active_leases = self
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?
            .active_leases;
        if active_leases != 0 {
            return Ok(());
        }
        let _observation = self.checkpoint_observation.lock().await;
        let target = self.checkpoint_target()?;
        let prefix =
            wallet_checkpoint_prefix(&self.pool, &self.nonce_pool, &self.schema_name).await?;
        self.checkpoint.recover(&target, &prefix).await
    }

    async fn prepare_checkpoint(
        &self,
        mutation: &ProviderMutation,
        successor_target: Option<CheckpointTarget>,
    ) -> Result<PreparedCheckpoint, ProviderTestError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let observation = self.checkpoint_observation.lock().await;
            let target = self.checkpoint_target()?;
            // The lock spans both role-specific reads and the external CAS. Once this
            // returns, the external authority retains the sole Prepared successor,
            // so no other mutation can commit between the two prefix halves.
            let prefix =
                wallet_checkpoint_prefix(&self.pool, &self.nonce_pool, &self.schema_name).await?;
            // A row-level collision after another successor committed is semantic,
            // not transient checkpoint contention; let the caller resolve it exactly.
            prefix.clone().exact_successor(mutation)?;
            match self
                .checkpoint
                .prepare(&target, prefix, mutation, successor_target.clone())
                .await
            {
                Ok(prepared) => return Ok(prepared),
                Err(error) if tokio::time::Instant::now() >= deadline => return Err(error),
                Err(_) => {
                    drop(observation);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    async fn acknowledge_checkpoint(
        &self,
        prepared: PreparedCheckpoint,
    ) -> Result<(), ProviderTestError> {
        let _observation = self.checkpoint_observation.lock().await;
        let target = self.checkpoint_target()?;
        let prefix =
            wallet_checkpoint_prefix(&self.pool, &self.nonce_pool, &self.schema_name).await?;
        self.checkpoint
            .acknowledge(&target, &prefix, prepared)
            .await
    }

    async fn abort_checkpoint(
        &self,
        prepared: PreparedCheckpoint,
    ) -> Result<(), ProviderTestError> {
        let _observation = self.checkpoint_observation.lock().await;
        let target = self.checkpoint_target()?;
        let prefix =
            wallet_checkpoint_prefix(&self.pool, &self.nonce_pool, &self.schema_name).await?;
        self.checkpoint.abort(&target, &prefix, prepared).await
    }

    async fn reconcile_checkpoint(
        &self,
        prepared: PreparedCheckpoint,
    ) -> Result<(), ProviderTestError> {
        let _observation = self.checkpoint_observation.lock().await;
        let target = self.checkpoint_target()?;
        let prefix =
            wallet_checkpoint_prefix(&self.pool, &self.nonce_pool, &self.schema_name).await?;
        let digest = prefix.digest()?;
        if digest == prepared.successor_digest {
            self.checkpoint
                .acknowledge(&target, &prefix, prepared)
                .await
        } else if digest == prepared.predecessor_digest {
            self.checkpoint.abort(&target, &prefix, prepared).await
        } else {
            Err(ProviderTestError::Invalid)
        }
    }

    async fn revoke(&self) -> Result<(), ProviderTestError> {
        {
            let mut state = self
                .mutable
                .lock()
                .map_err(|_| ProviderTestError::Unavailable)?;
            state.accepting = false;
            state.deployment_assemblies.clear();
        }
        loop {
            let drained = self
                .mutable
                .lock()
                .map_err(|_| ProviderTestError::Unavailable)?
                .active_leases
                == 0;
            if drained {
                return Ok(());
            }
            self.leases_changed.notified().await;
        }
    }

    async fn authorize_promotion(&self) -> Result<String, ProviderTestError> {
        {
            let mut state = self
                .mutable
                .lock()
                .map_err(|_| ProviderTestError::Unavailable)?;
            state.deployment_assemblies.prune(Instant::now());
            if state.accepting
                || state.active_leases != 0
                || !state.deployment_assemblies.is_empty()
            {
                return Err(ProviderTestError::Invalid);
            }
        }
        let _observation = self.checkpoint_observation.lock().await;
        let captured_prefix_digest =
            wallet_prefix_digest(&self.pool, &self.nonce_pool, &self.schema_name).await?;
        let mut state = self
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        state.deployment_assemblies.prune(Instant::now());
        if state.accepting || state.active_leases != 0 || !state.deployment_assemblies.is_empty() {
            return Err(ProviderTestError::Invalid);
        }
        state.captured_prefix_digest = Some(captured_prefix_digest.clone());
        state.promotion_authorized = true;
        Ok(captured_prefix_digest)
    }

    fn forget_hydrated_prefix(&self) -> Result<(), ProviderTestError> {
        let mut state = self
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        state.deployment_assemblies.prune(Instant::now());
        if state.accepting || state.active_leases != 0 || !state.deployment_assemblies.is_empty() {
            return Err(ProviderTestError::Invalid);
        }
        state.captured_prefix_digest = None;
        Ok(())
    }

    fn active_leases(&self) -> Result<usize, ProviderTestError> {
        let mut state = self
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        state.deployment_assemblies.prune(Instant::now());
        Ok(state.active_leases + state.deployment_assemblies.len())
    }

    fn successful_deployment_finishes(&self) -> u64 {
        self.successful_deployment_finishes.load(Ordering::Acquire)
    }

    fn arm_affine_lease_holds(&self, count: u64) -> Result<(), ProviderTestError> {
        if !(1..=16).contains(&count) {
            return Err(ProviderTestError::Invalid);
        }
        self.affine_lease_holds_remaining
            .compare_exchange(0, count, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| ProviderTestError::Invalid)
    }

    async fn apply_affine_lease_hold(&self) {
        let armed = self
            .affine_lease_holds_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok();
        if armed {
            self.entered_affine_lease_holds
                .fetch_add(1, Ordering::AcqRel);
            let _entered = EnteredAffineLeaseHold {
                count: &self.entered_affine_lease_holds,
            };
            tokio::time::sleep(LEASE_HOLD_FAULT_DURATION).await;
        }
    }

    async fn probe_context(&self, context: &ProviderTargetContext) -> bool {
        let Some((marker_class_id, marker_object_id)) =
            target_session_marker_lock_ids(&context.application_marker)
        else {
            return false;
        };
        if context.database_oid != self.database_oid || context.schema_name != self.schema_name {
            return false;
        }
        let row = sqlx::query(
            "SELECT EXISTS ( \
                 SELECT 1 FROM pg_catalog.pg_locks AS marker \
                 WHERE marker.pid = $1 AND marker.locktype = 'advisory' \
                   AND marker.database::bigint = $2 \
                   AND marker.classid::bigint = $3 AND marker.objid::bigint = $4 \
                   AND marker.objsubid = 2 AND marker.mode = 'ExclusiveLock' \
                   AND marker.granted \
             ) AS marker_matches, \
             CASE WHEN $5::text IS NULL THEN TRUE ELSE EXISTS ( \
                 SELECT 1 FROM pg_catalog.pg_locks AS retained_xid \
                 WHERE retained_xid.pid = $1 AND retained_xid.locktype = 'transactionid' \
                   AND retained_xid.transactionid::text = $5 AND retained_xid.granted \
             ) END AS transaction_matches",
        )
        .bind(context.backend_pid)
        .bind(i64::from(context.database_oid))
        .bind(marker_class_id)
        .bind(marker_object_id)
        .bind(context.transaction_id.map(|value| value.to_string()))
        .fetch_one(&self.pool)
        .await;
        let Ok(row) = row else {
            return false;
        };
        row.try_get::<bool, _>("marker_matches").unwrap_or(false)
            && row
                .try_get::<bool, _>("transaction_matches")
                .unwrap_or(false)
    }

    async fn write_transaction_terminated(&self, context: &ProviderTargetContext) -> bool {
        let Some(expected_transaction_id) = context.transaction_id else {
            return false;
        };
        if context.database_oid != self.database_oid
            || context.schema_name != self.schema_name
            || context.snapshot_id.is_some()
        {
            return false;
        }
        let Some((marker_class_id, marker_object_id)) =
            target_session_marker_lock_ids(&context.application_marker)
        else {
            return false;
        };
        let retained = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS ( \
                 SELECT 1 FROM pg_catalog.pg_locks AS marker \
                 WHERE marker.pid = $1 AND marker.locktype = 'advisory' \
                   AND marker.database::bigint = $2 \
                   AND marker.classid::bigint = $3 AND marker.objid::bigint = $4 \
                   AND marker.objsubid = 2 AND marker.mode = 'ExclusiveLock' \
                   AND marker.granted AND EXISTS ( \
                       SELECT 1 FROM pg_catalog.pg_locks AS retained_xid \
                       WHERE retained_xid.pid = marker.pid \
                         AND retained_xid.locktype = 'transactionid' \
                         AND retained_xid.transactionid::text = $5 \
                         AND retained_xid.granted \
                   ) \
             )",
        )
        .bind(context.backend_pid)
        .bind(i64::from(context.database_oid))
        .bind(marker_class_id)
        .bind(marker_object_id)
        .bind(expected_transaction_id.to_string())
        .fetch_one(&self.pool)
        .await;
        retained.is_ok_and(|retained| !retained)
    }

    fn acquire_lease(
        self: &Arc<Self>,
        expected_incarnation: &WalletNonceStoreIncarnation,
        expected_provider_head: &EvmWalletReference,
    ) -> Result<ActiveLeaseGuard, ProviderTestError> {
        let mut state = self
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        if !state.accepting
            || &state.current_incarnation != expected_incarnation
            || &state.provider_fence_head_ref != expected_provider_head
        {
            return Err(ProviderTestError::Unavailable);
        }
        state.active_leases += 1;
        Ok(ActiveLeaseGuard {
            state: Arc::clone(self),
            active: true,
        })
    }

    fn acquire_issuance_lease(
        self: &Arc<Self>,
        expected_incarnation: &WalletNonceStoreIncarnation,
        expected_provider_head: &EvmWalletReference,
    ) -> Result<ActiveLeaseGuard, ProviderTestError> {
        let incarnation_ref = canonical_wallet_reference(expected_incarnation)
            .map_err(|_| ProviderTestError::Invalid)?;
        let mut state = self
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        if !state.accepting
            || self
                .allowed_issuance_incarnations
                .get(incarnation_ref.content_digest())
                != Some(expected_incarnation)
            || &state.provider_fence_head_ref != expected_provider_head
        {
            return Err(ProviderTestError::Unavailable);
        }
        state.active_leases += 1;
        Ok(ActiveLeaseGuard {
            state: Arc::clone(self),
            active: true,
        })
    }
}

struct ActiveLeaseGuard {
    state: Arc<ProviderState>,
    active: bool,
}

struct EnteredAffineLeaseHold<'a> {
    count: &'a AtomicU64,
}

impl Drop for EnteredAffineLeaseHold<'_> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for ActiveLeaseGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Ok(mut state) = self.state.mutable.lock() {
            state.active_leases = state.active_leases.saturating_sub(1);
        }
        self.state.leases_changed.notify_one();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderTargetContext {
    database_oid: u32,
    backend_pid: i32,
    transaction_id: Option<u32>,
    snapshot_id: Option<String>,
    schema_name: String,
    application_marker: String,
    store_incarnation: WalletNonceStoreIncarnation,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedMutationProof {
    provider_id: String,
    challenge: String,
    context: ProviderTargetContext,
    operation_key: String,
    payload_digest: String,
    signature: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProviderRequest {
    Authenticate {
        version: u16,
        provider_id: String,
        fence_lineage_ref: EvmWalletReference,
        chain_registry_lineage_ref: EvmWalletReference,
        current_chain_registry_head_ref: EvmWalletReference,
        challenge: String,
    },
    QualifyRoutingCatalog {
        descriptor: EvmRoutingCatalogDescriptor,
        current_chain_registry_head_ref: EvmWalletReference,
    },
    BeginDeploymentAssembly {
        descriptor: EvmRoutingCatalogDescriptor,
        provider_fence_head_ref: EvmWalletReference,
        binding: DeploymentAssemblyBindingWire,
    },
    FinishDeploymentAssembly {
        descriptor: EvmRoutingCatalogDescriptor,
        provider_fence_head_ref: EvmWalletReference,
        binding: DeploymentAssemblyBindingWire,
        assembly_lease: String,
        checkpoint: String,
        exchange_ref: String,
        proofs: Vec<DeploymentAssemblyRouteProofWire>,
    },
    QualifyDomain {
        attestation: WalletNonceDomainActivationAttestation,
        store_incarnation: WalletNonceStoreIncarnation,
        current_public_lineage_head: HistoryObject,
    },
    BeginRead {
        registry_issuance_ref: EvmWalletReference,
        store_incarnation: WalletNonceStoreIncarnation,
        provider_fence_head_ref: EvmWalletReference,
    },
    BindRead {
        context: ProviderTargetContext,
    },
    FinishRead,
    BeginResolution {
        context: ProviderTargetContext,
        operation_key: String,
    },
    BindResolution {
        context: ProviderTargetContext,
        operation_key: String,
    },
    FinishResolution,
    BeginWrite {
        registry_issuance_ref: EvmWalletReference,
        store_incarnation: WalletNonceStoreIncarnation,
        provider_fence_head_ref: EvmWalletReference,
        state_input_ref: LexicalValueRef,
        operation_key: String,
    },
    BindWrite {
        context: ProviderTargetContext,
        operation_key: String,
    },
    RevalidateWrite {
        context: ProviderTargetContext,
        operation_key: String,
    },
    PrepareMutation {
        context: ProviderTargetContext,
        operation_key: String,
        mutation: Box<ProviderMutation>,
    },
    FinishWrite {
        database_commit_observed: bool,
        operation_key: String,
    },
    OpenRegistryMaintenance,
    PrepareIssuance {
        record: WalletNonceDomainActivationRecord,
        observed_incarnation: WalletNonceStoreIncarnation,
        context: ProviderTargetContext,
        registry_lineage_ref: EvmWalletReference,
        provider_fence_head_ref: EvmWalletReference,
    },
    ConfirmIssuance {
        attestation: WalletNonceDomainActivationAttestation,
    },
    PreparePromotion {
        successor: WalletNonceStoreSuccessor,
        context: ProviderTargetContext,
        registry_lineage_ref: EvmWalletReference,
        provider_fence_head_ref: EvmWalletReference,
    },
    ConfirmPromotion {
        attestation: serde_json::Value,
    },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProviderReply {
    Authenticated {
        signature: String,
    },
    DomainQualified {
        provider_fence_head_ref: EvmWalletReference,
        target_attestation: String,
    },
    RoutingCatalogQualified {
        chain_registry_head_ref: EvmWalletReference,
        provider_fence_head_ref: EvmWalletReference,
        target_attestation: String,
    },
    DeploymentAssemblyBegun {
        assembly_lease: String,
        checkpoint: String,
        finish_authorization_commitment: String,
        route_challenges: Vec<DeploymentAssemblyRouteChallengeWire>,
        provider_attestation: String,
    },
    DeploymentAssemblyFinished {
        finish_authorization: Zeroizing<String>,
        provider_attestation: String,
    },
    RegistryMaintenanceOpened {
        registry_lineage_ref: EvmWalletReference,
        provider_fence_head_ref: EvmWalletReference,
        provider_attestation: String,
    },
    LeaseChallenge {
        marker: String,
        provider_attestation: String,
    },
    Current {
        target_attestation: String,
    },
    MutationPrepared {
        provider_attestation: String,
    },
    Superseded {
        evidence: WalletNonceStoreLineageHead,
        public_head: HistoryObject,
    },
    Finished {
        provider_attestation: String,
    },
    IssuancePrepared {
        registry_issuance_ref: EvmWalletReference,
        provider_attestation: String,
    },
    PromotionPrepared {
        provider_attestation: String,
    },
    Confirmed {
        provider_attestation: String,
    },
    Unavailable,
    EntryUnknown,
    Conflict,
    Rejected,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentAssemblyRouteChallengeWire {
    route_generation_ref: ContentRef,
    target_identity: String,
    route_challenge: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentAssemblyRouteProofWire {
    ordinal: u16,
    route_generation_ref: ContentRef,
    proof: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentAssemblyBindingWire {
    activation: WalletNonceDomainActivationAttestation,
    store_incarnation: WalletNonceStoreIncarnation,
    current_public_lineage_head: HistoryObject,
    wallet_provider_fence_head_ref: EvmWalletReference,
    semantic_signer_id: StableId,
    signer_generation_ref: ContentRef,
    signer_fence_ref: ContentRef,
    direct_sign_exclusion_ref: ContentRef,
    semantic_contract_refs: Vec<ContentRef>,
    release_history_digests: Vec<ContentDigest>,
}

async fn serve_connection(
    stream: UnixStream,
    state: Arc<ProviderState>,
    key_pair: Arc<Ed25519KeyPair>,
) -> Result<(), ProviderTestError> {
    let (read, write) = tokio::io::split(stream);
    let mut channel = ServerChannel {
        read: BufReader::new(read),
        write,
        authentication_challenge: [0; 32],
        key_pair,
    };
    let request: ProviderRequest = channel.receive().await?;
    let ProviderRequest::Authenticate {
        version,
        provider_id,
        fence_lineage_ref,
        chain_registry_lineage_ref,
        current_chain_registry_head_ref,
        mut challenge,
    } = request
    else {
        return Err(ProviderTestError::Invalid);
    };
    if version != PROTOCOL_VERSION
        || provider_id != state.provider_id
        || fence_lineage_ref != state.fence_lineage_ref
        || chain_registry_lineage_ref != state.chain_registry_lineage_ref
        || current_chain_registry_head_ref != state.current_chain_registry_head_ref
    {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    }
    let challenge_bytes = hex::decode(&challenge).map_err(|_| ProviderTestError::Invalid)?;
    challenge.zeroize();
    channel.authentication_challenge = challenge_bytes
        .try_into()
        .map_err(|_| ProviderTestError::Invalid)?;
    let signature = channel.authentication_signature(&state)?;
    channel
        .send(&ProviderReply::Authenticated { signature })
        .await?;

    let request = channel.receive::<ProviderRequest>().await?;
    match request {
        ProviderRequest::QualifyRoutingCatalog {
            descriptor,
            current_chain_registry_head_ref,
        } => {
            state
                .routing_qualification_calls
                .fetch_add(1, Ordering::AcqRel);
            descriptor
                .content_ref()
                .map_err(|_| ProviderTestError::Invalid)?;
            let allowed = descriptor == state.current_routing_catalog
                && current_chain_registry_head_ref == state.current_chain_registry_head_ref
                && descriptor.chain_registry_head_ref() == &state.current_chain_registry_head_ref
                && descriptor.chain_instances().iter().all(|attestation| {
                    attestation.declaration().chain_registry_lineage_ref()
                        == &state.chain_registry_lineage_ref
                });
            let provider_head = state
                .mutable
                .lock()
                .map_err(|_| ProviderTestError::Unavailable)?
                .provider_fence_head_ref
                .clone();
            if !allowed {
                channel.send(&ProviderReply::Rejected).await?;
                return Ok(());
            }
            let chain_registry_head_ref = descriptor.chain_registry_head_ref().clone();
            let signature = channel.assertion_signature(
                "qualify-routing-catalog",
                &(
                    &descriptor,
                    &current_chain_registry_head_ref,
                    &chain_registry_head_ref,
                    &provider_head,
                ),
            )?;
            channel
                .send(&ProviderReply::RoutingCatalogQualified {
                    chain_registry_head_ref,
                    provider_fence_head_ref: provider_head,
                    target_attestation: signature,
                })
                .await
        }
        ProviderRequest::BeginDeploymentAssembly {
            descriptor,
            provider_fence_head_ref,
            binding,
        } => {
            if descriptor != state.current_routing_catalog
                || descriptor.generations().is_empty()
                || descriptor.generations().len() > 64
            {
                channel.send(&ProviderReply::Rejected).await?;
                return Ok(());
            }
            let random_assembly_lease = *random_bytes::<32>()?;
            let assembly_lease_hex = state
                .mutable
                .lock()
                .map_err(|_| ProviderTestError::Unavailable)?
                .hostile_deployment_assembly_lease_sequence
                .pop_front()
                .unwrap_or_else(|| hex::encode(random_assembly_lease));
            let checkpoint = *random_bytes::<32>()?;
            let finish_authorization = random_bytes::<32>()?;
            let finish_authorization_commitment =
                *mfm_canonical::sha256_digest_bytes(finish_authorization.as_ref()).as_bytes();
            let mut route_challenges = Vec::with_capacity(descriptor.generations().len());
            for generation in descriptor.generations() {
                let route_generation_ref = generation
                    .generation_ref()
                    .map_err(|_| ProviderTestError::Invalid)?
                    .to_content_ref()
                    .map_err(|_| ProviderTestError::Invalid)?;
                let target = state
                    .rpc_inventory_targets
                    .get(&route_generation_ref)
                    .ok_or(ProviderTestError::Invalid)?;
                route_challenges.push(DeploymentAssemblyRouteChallengeWire {
                    route_generation_ref,
                    target_identity: hex::encode(target.target_identity),
                    route_challenge: hex::encode(random_bytes::<32>()?.as_ref()),
                });
            }
            let checkpoint_hex = hex::encode(checkpoint);
            let finish_authorization_commitment_hex = hex::encode(finish_authorization_commitment);
            let signature = channel.assertion_signature(
                "begin-deployment-assembly",
                &(
                    &descriptor,
                    &provider_fence_head_ref,
                    &binding,
                    &assembly_lease_hex,
                    &checkpoint_hex,
                    &finish_authorization_commitment_hex,
                    &route_challenges,
                ),
            )?;
            let inserted = {
                let mut retained = state
                    .mutable
                    .lock()
                    .map_err(|_| ProviderTestError::Unavailable)?;
                let now = Instant::now();
                retained.deployment_assemblies.prune(now);
                if retained.deployment_assemblies.len() >= MAX_DEPLOYMENT_ASSEMBLY_LEASES
                    || !retained.accepting
                    || descriptor != state.current_routing_catalog
                    || provider_fence_head_ref != retained.provider_fence_head_ref
                    || !deployment_binding_matches(&state, &retained, &binding)
                {
                    false
                } else {
                    retained.deployment_assemblies.insert(
                        assembly_lease_hex.clone(),
                        ProviderDeploymentAssembly {
                            descriptor,
                            provider_fence_head_ref,
                            binding,
                            checkpoint,
                            finish_authorization: Zeroizing::new(finish_authorization.to_vec()),
                            challenges: route_challenges.clone(),
                        },
                        now + DEPLOYMENT_ASSEMBLY_LEASE_TTL,
                    )
                }
            };
            if !inserted {
                channel.send(&ProviderReply::Unavailable).await?;
                return Ok(());
            }
            channel
                .send(&ProviderReply::DeploymentAssemblyBegun {
                    assembly_lease: assembly_lease_hex,
                    checkpoint: checkpoint_hex,
                    finish_authorization_commitment: finish_authorization_commitment_hex,
                    route_challenges,
                    provider_attestation: signature,
                })
                .await
        }
        ProviderRequest::FinishDeploymentAssembly {
            descriptor,
            provider_fence_head_ref,
            binding,
            assembly_lease,
            checkpoint,
            exchange_ref,
            proofs,
        } => {
            let pending = {
                let mut retained = state
                    .mutable
                    .lock()
                    .map_err(|_| ProviderTestError::Unavailable)?;
                let now = Instant::now();
                retained.deployment_assemblies.prune(now);
                let valid = retained
                    .deployment_assemblies
                    .get(&assembly_lease)
                    .is_some_and(|pending| {
                        retained.accepting
                            && exact_affine_snapshot(
                                &state.current_routing_catalog,
                                &pending.descriptor,
                                &descriptor,
                            )
                            && exact_affine_snapshot(
                                &retained.provider_fence_head_ref,
                                &pending.provider_fence_head_ref,
                                &provider_fence_head_ref,
                            )
                            && pending.binding == binding
                            && deployment_binding_matches(&state, &retained, &binding)
                            && hex::encode(pending.checkpoint) == checkpoint
                            && validate_deployment_proofs(
                                &state,
                                pending,
                                &assembly_lease,
                                &proofs,
                                &exchange_ref,
                            )
                            .is_ok()
                    });
                valid
                    .then(|| retained.deployment_assemblies.remove(&assembly_lease))
                    .flatten()
            };
            let Some(pending) = pending else {
                channel.send(&ProviderReply::Rejected).await?;
                return Ok(());
            };
            state
                .successful_deployment_finishes
                .fetch_add(1, Ordering::AcqRel);
            let finish_authorization =
                Zeroizing::new(hex::encode(pending.finish_authorization.as_slice()));
            let signature = channel.assertion_signature(
                "finish-deployment-assembly",
                &(
                    &descriptor,
                    &provider_fence_head_ref,
                    &binding,
                    &assembly_lease,
                    &checkpoint,
                    &exchange_ref,
                    &proofs,
                    finish_authorization.as_str(),
                ),
            )?;
            channel
                .send(&ProviderReply::DeploymentAssemblyFinished {
                    finish_authorization,
                    provider_attestation: signature,
                })
                .await
        }
        ProviderRequest::QualifyDomain {
            attestation,
            store_incarnation,
            current_public_lineage_head,
        } => {
            state.recover_checkpoint_if_idle().await?;
            let provider_head = {
                let retained = state
                    .mutable
                    .lock()
                    .map_err(|_| ProviderTestError::Unavailable)?;
                let issued = retained.issued.get(
                    attestation
                        .current_schema_record
                        .wallet_nonce_domain
                        .as_str(),
                );
                if !retained.accepting
                    || issued != Some(&attestation)
                    || retained.current_incarnation != store_incarnation
                    || retained.current_public_lineage_head != current_public_lineage_head
                {
                    None
                } else {
                    Some(retained.provider_fence_head_ref.clone())
                }
            };
            let Some(provider_head) = provider_head else {
                channel.send(&ProviderReply::Rejected).await?;
                return Ok(());
            };
            let signature = channel.assertion_signature(
                "qualify-domain",
                &(
                    &attestation,
                    &store_incarnation,
                    &current_public_lineage_head,
                    &provider_head,
                ),
            )?;
            channel
                .send(&ProviderReply::DomainQualified {
                    provider_fence_head_ref: provider_head,
                    target_attestation: signature,
                })
                .await
        }
        ProviderRequest::OpenRegistryMaintenance => {
            let provider_head = state
                .mutable
                .lock()
                .map_err(|_| ProviderTestError::Unavailable)?
                .provider_fence_head_ref
                .clone();
            let signature = channel.assertion_signature(
                "open-registry-maintenance",
                &(&state.registry_lineage_ref, &provider_head),
            )?;
            channel
                .send(&ProviderReply::RegistryMaintenanceOpened {
                    registry_lineage_ref: state.registry_lineage_ref.clone(),
                    provider_fence_head_ref: provider_head,
                    provider_attestation: signature,
                })
                .await
        }
        ProviderRequest::BeginRead {
            registry_issuance_ref,
            store_incarnation,
            provider_fence_head_ref,
        } => {
            handle_read(
                &mut channel,
                state,
                registry_issuance_ref,
                store_incarnation,
                provider_fence_head_ref,
            )
            .await
        }
        ProviderRequest::BeginWrite {
            registry_issuance_ref,
            store_incarnation,
            provider_fence_head_ref,
            state_input_ref,
            operation_key,
        } => {
            handle_write(
                &mut channel,
                state,
                registry_issuance_ref,
                store_incarnation,
                provider_fence_head_ref,
                state_input_ref,
                operation_key,
            )
            .await
        }
        ProviderRequest::PrepareIssuance {
            record,
            observed_incarnation,
            context,
            registry_lineage_ref,
            provider_fence_head_ref,
        } => {
            handle_issuance(
                &mut channel,
                state,
                record,
                observed_incarnation,
                context,
                registry_lineage_ref,
                provider_fence_head_ref,
            )
            .await
        }
        ProviderRequest::PreparePromotion {
            successor,
            context,
            registry_lineage_ref,
            provider_fence_head_ref,
        } => {
            handle_promotion(
                &mut channel,
                state,
                successor,
                context,
                registry_lineage_ref,
                provider_fence_head_ref,
            )
            .await
        }
        _ => channel.send(&ProviderReply::Rejected).await,
    }
}

fn validate_deployment_proofs(
    state: &ProviderState,
    pending: &ProviderDeploymentAssembly,
    assembly_lease: &str,
    proofs: &[DeploymentAssemblyRouteProofWire],
    exchange_ref: &str,
) -> Result<(), ProviderTestError> {
    if proofs.len() != pending.challenges.len() {
        return Err(ProviderTestError::Invalid);
    }
    let checkpoint = hex::encode(pending.checkpoint);
    let finish_commitment =
        hex::encode(mfm_canonical::sha256_digest_bytes(&pending.finish_authorization).as_bytes());
    let mut exchange_bytes = Vec::new();
    for (index, (proof, challenge)) in proofs.iter().zip(&pending.challenges).enumerate() {
        if usize::from(proof.ordinal) != index
            || proof.route_generation_ref != challenge.route_generation_ref
        {
            return Err(ProviderTestError::Invalid);
        }
        let proof_bytes = decode_canonical_bounded_hex(&proof.proof, 16 * 1024)?;
        let target = state
            .rpc_inventory_targets
            .get(&challenge.route_generation_ref)
            .ok_or(ProviderTestError::Invalid)?;
        if challenge.target_identity != hex::encode(target.target_identity) {
            return Err(ProviderTestError::Invalid);
        }
        let proof_message = serde_json::to_vec(&(
            proof.ordinal,
            &challenge.route_generation_ref,
            &challenge.target_identity,
            assembly_lease,
            &checkpoint,
            &challenge.route_challenge,
            &finish_commitment,
        ))
        .map_err(|_| ProviderTestError::Invalid)?;
        let key = hmac::Key::new(hmac::HMAC_SHA256, target.proof_key.as_ref());
        hmac::verify(&key, &proof_message, &proof_bytes).map_err(|_| ProviderTestError::Invalid)?;

        append_deployment_frame(&mut exchange_bytes, &proof.ordinal.to_be_bytes())?;
        append_deployment_frame(
            &mut exchange_bytes,
            &serde_json::to_vec(&challenge.route_generation_ref)
                .map_err(|_| ProviderTestError::Invalid)?,
        )?;
        append_deployment_frame(
            &mut exchange_bytes,
            &hex::decode(&challenge.target_identity).map_err(|_| ProviderTestError::Invalid)?,
        )?;
        append_deployment_frame(
            &mut exchange_bytes,
            &hex::decode(assembly_lease).map_err(|_| ProviderTestError::Invalid)?,
        )?;
        append_deployment_frame(
            &mut exchange_bytes,
            &hex::decode(&checkpoint).map_err(|_| ProviderTestError::Invalid)?,
        )?;
        append_deployment_frame(
            &mut exchange_bytes,
            &hex::decode(&challenge.route_challenge).map_err(|_| ProviderTestError::Invalid)?,
        )?;
        append_deployment_frame(
            &mut exchange_bytes,
            &hex::decode(&finish_commitment).map_err(|_| ProviderTestError::Invalid)?,
        )?;
        append_deployment_frame(&mut exchange_bytes, &proof_bytes)?;
    }
    if exchange_ref != hex::encode(mfm_canonical::sha256_digest_bytes(&exchange_bytes).as_bytes()) {
        return Err(ProviderTestError::Invalid);
    }
    Ok(())
}

fn deployment_binding_matches(
    state: &ProviderState,
    retained: &MutableProviderState,
    binding: &DeploymentAssemblyBindingWire,
) -> bool {
    let policy = state
        .deployment_assembly_policies
        .get(&binding.semantic_signer_id);
    binding.activation.validate().is_ok()
        && binding.store_incarnation.validate().is_ok()
        && binding.current_public_lineage_head.validate().is_ok()
        && binding.wallet_provider_fence_head_ref == retained.provider_fence_head_ref
        && binding.store_incarnation == retained.current_incarnation
        && binding.current_public_lineage_head == retained.current_public_lineage_head
        && retained
            .issued
            .values()
            .any(|issued| issued == &binding.activation)
        && policy.is_some_and(|policy| deployment_policy_matches(policy, binding))
        && binding
            .activation
            .current_schema_record
            .chain_instance_attestation
            .binding()
            .is_ok_and(|chain| {
                state
                    .current_routing_catalog
                    .chain_instances()
                    .iter()
                    .any(|attestation| attestation.binding().is_ok_and(|item| item == chain))
            })
}

fn deployment_policy_matches(
    policy: &ProviderDeploymentAssemblyPolicy,
    binding: &DeploymentAssemblyBindingWire,
) -> bool {
    deployment_policy_fields_match(
        policy,
        &binding.semantic_signer_id,
        &binding.signer_generation_ref,
        &binding.signer_fence_ref,
        &binding.direct_sign_exclusion_ref,
        &binding.semantic_contract_refs,
        &binding.release_history_digests,
    )
}

fn deployment_policy_fields_match(
    policy: &ProviderDeploymentAssemblyPolicy,
    semantic_signer_id: &StableId,
    signer_generation_ref: &ContentRef,
    signer_fence_ref: &ContentRef,
    direct_sign_exclusion_ref: &ContentRef,
    semantic_contract_refs: &[ContentRef],
    release_history_digests: &[ContentDigest],
) -> bool {
    &policy.semantic_signer_id == semantic_signer_id
        && &policy.signer_generation_ref == signer_generation_ref
        && &policy.signer_fence_ref == signer_fence_ref
        && &policy.direct_sign_exclusion_ref == direct_sign_exclusion_ref
        && policy.semantic_contract_refs == semantic_contract_refs
        && policy.release_history_digests == release_history_digests
}

fn deployment_lease_is_current(expires_at: Instant, now: Instant) -> bool {
    expires_at > now
}

fn exact_affine_snapshot<T: PartialEq>(current: &T, pending: &T, requested: &T) -> bool {
    current == pending && pending == requested
}

fn decode_canonical_bounded_hex(
    value: &str,
    maximum_bytes: usize,
) -> Result<Vec<u8>, ProviderTestError> {
    if value.is_empty()
        || value.len() > maximum_bytes.saturating_mul(2)
        || !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ProviderTestError::Invalid);
    }
    hex::decode(value).map_err(|_| ProviderTestError::Invalid)
}

fn append_deployment_frame(output: &mut Vec<u8>, value: &[u8]) -> Result<(), ProviderTestError> {
    let length = u32::try_from(value.len()).map_err(|_| ProviderTestError::Invalid)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

async fn handle_read(
    channel: &mut ServerChannel,
    state: Arc<ProviderState>,
    issuance_ref: EvmWalletReference,
    incarnation: WalletNonceStoreIncarnation,
    provider_head: EvmWalletReference,
) -> Result<(), ProviderTestError> {
    if !issued_reference_matches(&state, &issuance_ref, &incarnation, &provider_head)? {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    }
    let marker = random_marker()?;
    let signature = channel.assertion_signature(
        "begin-read",
        &(&issuance_ref, &incarnation, &provider_head, &marker),
    )?;
    channel
        .send(&ProviderReply::LeaseChallenge {
            marker: marker.clone(),
            provider_attestation: signature,
        })
        .await?;
    let ProviderRequest::BindRead { context } = channel.receive().await? else {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    };
    if context.application_marker != marker || !state.probe_context(&context).await {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    }
    let _lease = match state.acquire_lease(&incarnation, &provider_head) {
        Ok(lease) => lease,
        Err(_) => {
            send_superseded_or_unavailable(channel, &state).await?;
            return Ok(());
        }
    };
    let signature = channel.assertion_signature("bind-read", &context)?;
    channel
        .send(&ProviderReply::Current {
            target_attestation: signature,
        })
        .await?;
    state.apply_affine_lease_hold().await;
    let ProviderRequest::FinishRead = channel.receive().await? else {
        channel.send(&ProviderReply::EntryUnknown).await?;
        return Ok(());
    };
    let signature = channel.assertion_signature("finish-read", &context)?;
    channel
        .send(&ProviderReply::Finished {
            provider_attestation: signature,
        })
        .await
}

#[allow(clippy::too_many_arguments)]
async fn handle_write(
    channel: &mut ServerChannel,
    state: Arc<ProviderState>,
    issuance_ref: EvmWalletReference,
    incarnation: WalletNonceStoreIncarnation,
    provider_head: EvmWalletReference,
    state_input_ref: LexicalValueRef,
    operation_key: String,
) -> Result<(), ProviderTestError> {
    if !issued_reference_matches(&state, &issuance_ref, &incarnation, &provider_head)? {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    }
    let marker = random_marker()?;
    let signature = channel.assertion_signature(
        "begin-write",
        &(
            &issuance_ref,
            &incarnation,
            &provider_head,
            &state_input_ref,
            &operation_key,
            &marker,
        ),
    )?;
    channel
        .send(&ProviderReply::LeaseChallenge {
            marker: marker.clone(),
            provider_attestation: signature,
        })
        .await?;
    let ProviderRequest::BindWrite {
        context,
        operation_key: bound_key,
    } = channel.receive().await?
    else {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    };
    if bound_key != operation_key
        || context.application_marker != marker
        || !state.probe_context(&context).await
    {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    }
    let _lease = match state.acquire_lease(&incarnation, &provider_head) {
        Ok(lease) => lease,
        Err(_) => {
            send_superseded_or_unavailable(channel, &state).await?;
            return Ok(());
        }
    };
    let signature = channel.assertion_signature("bind-write", &(&context, &operation_key))?;
    channel
        .send(&ProviderReply::Current {
            target_attestation: signature,
        })
        .await?;
    state.apply_affine_lease_hold().await;
    let request = channel.receive::<ProviderRequest>().await?;
    match request {
        ProviderRequest::FinishWrite {
            database_commit_observed: false,
            operation_key: finished_key,
        } if finished_key == operation_key => {
            finish_write(channel, &state, &context, &operation_key, false, None).await
        }
        ProviderRequest::RevalidateWrite {
            context: retained_context,
            operation_key: retained_key,
        } if retained_context == context && retained_key == operation_key => {
            if !state
                .mutable
                .lock()
                .map_err(|_| ProviderTestError::Unavailable)?
                .accepting
            {
                send_superseded_or_unavailable(channel, &state).await?;
                return match channel.receive::<ProviderRequest>().await? {
                    ProviderRequest::FinishWrite {
                        database_commit_observed: false,
                        operation_key: finished_key,
                    } if finished_key == operation_key => {
                        finish_write(channel, &state, &context, &operation_key, false, None).await
                    }
                    _ => channel.send(&ProviderReply::EntryUnknown).await,
                };
            }
            let signature =
                channel.assertion_signature("revalidate-write", &(&context, &operation_key))?;
            channel
                .send(&ProviderReply::Current {
                    target_attestation: signature,
                })
                .await?;
            let request = channel.receive::<ProviderRequest>().await?;
            let (request, prepared) = match request {
                ProviderRequest::PrepareMutation {
                    context: retained_context,
                    operation_key: retained_key,
                    mutation,
                } if retained_context == context && retained_key == operation_key => {
                    let signature = channel.assertion_signature(
                        "prepare-mutation",
                        &(&context, &operation_key, &mutation),
                    )?;
                    let provider_attestation = channel.persisted_mutation_attestation(
                        &state.provider_id,
                        &context,
                        &operation_key,
                        mutation.as_ref(),
                        &signature,
                    )?;
                    let checkpoint_mutation = checkpoint_mutation_with_provider_attestation(
                        mutation.as_ref(),
                        &provider_attestation,
                    )?;
                    let prepared = match state.prepare_checkpoint(&checkpoint_mutation, None).await
                    {
                        Ok(prepared) => prepared,
                        Err(_) => {
                            channel.send(&ProviderReply::Rejected).await?;
                            return Ok(());
                        }
                    };
                    if channel
                        .send(&ProviderReply::MutationPrepared {
                            provider_attestation,
                        })
                        .await
                        .is_err()
                    {
                        let _ = state.abort_checkpoint(prepared).await;
                        return Ok(());
                    }
                    match channel.receive::<ProviderRequest>().await {
                        Ok(request) => (request, Some(prepared)),
                        Err(error) => return Err(error),
                    }
                }
                request => (request, None),
            };
            match request {
                ProviderRequest::FinishWrite {
                    database_commit_observed,
                    operation_key: finished_key,
                } if finished_key == operation_key => {
                    finish_write(
                        channel,
                        &state,
                        &context,
                        &operation_key,
                        database_commit_observed,
                        prepared,
                    )
                    .await
                }
                ProviderRequest::BeginResolution {
                    context: retained_context,
                    operation_key: retained_key,
                } if retained_context == context && retained_key == operation_key => {
                    handle_resolution(channel, &state, &context, &operation_key, prepared).await
                }
                _ => channel.send(&ProviderReply::EntryUnknown).await,
            }
        }
        _ => channel.send(&ProviderReply::Rejected).await,
    }
}

async fn finish_write(
    channel: &mut ServerChannel,
    state: &ProviderState,
    context: &ProviderTargetContext,
    operation_key: &str,
    database_commit_observed: bool,
    prepared: Option<PreparedCheckpoint>,
) -> Result<(), ProviderTestError> {
    if let Some(prepared) = prepared {
        let checkpoint_result = if database_commit_observed {
            state.acknowledge_checkpoint(prepared).await
        } else {
            state.abort_checkpoint(prepared).await
        };
        if checkpoint_result.is_err() {
            channel.send(&ProviderReply::EntryUnknown).await?;
            return Ok(());
        }
    }
    let signature = channel.assertion_signature(
        "finish-write",
        &(context, operation_key, database_commit_observed),
    )?;
    channel
        .send(&ProviderReply::Finished {
            provider_attestation: signature,
        })
        .await
}

async fn handle_resolution(
    channel: &mut ServerChannel,
    state: &ProviderState,
    write_context: &ProviderTargetContext,
    operation_key: &str,
    prepared: Option<PreparedCheckpoint>,
) -> Result<(), ProviderTestError> {
    if !state.write_transaction_terminated(write_context).await {
        channel.send(&ProviderReply::EntryUnknown).await?;
        let terminated = tokio::time::timeout(Duration::from_secs(15), async {
            while !state.write_transaction_terminated(write_context).await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok();
        if terminated {
            if let Some(prepared) = prepared {
                if state.reconcile_checkpoint(prepared).await.is_err() {
                    // The caller already received EntryUnknown, so no later response can
                    // safely report a checkpoint divergence. Fence the target instead of
                    // accepting work against an unresolved committed prefix.
                    state
                        .mutable
                        .lock()
                        .map_err(|_| ProviderTestError::Unavailable)?
                        .accepting = false;
                }
            }
        }
        return Ok(());
    }
    if let Some(prepared) = prepared {
        if state.reconcile_checkpoint(prepared).await.is_err() {
            channel.send(&ProviderReply::EntryUnknown).await?;
            return Ok(());
        }
    }
    let marker = random_marker()?;
    let signature = channel
        .assertion_signature("begin-resolution", &(write_context, operation_key, &marker))?;
    channel
        .send(&ProviderReply::LeaseChallenge {
            marker: marker.clone(),
            provider_attestation: signature,
        })
        .await?;
    let ProviderRequest::BindResolution {
        context,
        operation_key: retained_key,
    } = channel.receive().await?
    else {
        channel.send(&ProviderReply::EntryUnknown).await?;
        return Ok(());
    };
    if retained_key != operation_key
        || context.application_marker != marker
        || !state.probe_context(&context).await
    {
        channel.send(&ProviderReply::EntryUnknown).await?;
        return Ok(());
    }
    let signature = channel.assertion_signature("bind-resolution", &(&context, operation_key))?;
    channel
        .send(&ProviderReply::Current {
            target_attestation: signature,
        })
        .await?;
    let ProviderRequest::FinishResolution = channel.receive().await? else {
        channel.send(&ProviderReply::EntryUnknown).await?;
        return Ok(());
    };
    let signature = channel.assertion_signature("finish-resolution", &context)?;
    channel
        .send(&ProviderReply::Finished {
            provider_attestation: signature,
        })
        .await
}

#[allow(clippy::too_many_arguments)]
async fn handle_issuance(
    channel: &mut ServerChannel,
    state: Arc<ProviderState>,
    record: WalletNonceDomainActivationRecord,
    incarnation: WalletNonceStoreIncarnation,
    context: ProviderTargetContext,
    registry_lineage_ref: EvmWalletReference,
    provider_head: EvmWalletReference,
) -> Result<(), ProviderTestError> {
    let domain_id = record.wallet_nonce_domain.as_str().to_owned();
    let record_ref = canonical_wallet_reference(&record).map_err(|_| ProviderTestError::Invalid)?;
    let allowed = state
        .allowed_activation_records
        .get(record_ref.content_digest())
        == Some(&record)
        && catalog_admits_activation(&state.current_routing_catalog, &record);
    let issuance_ref = provider_issuance_reference(&state.provider_id, &record_ref)?;
    let expected = WalletNonceDomainActivationAttestation {
        activation_record_ref: record_ref,
        registry_issuance_ref: issuance_ref.clone(),
        activation_registry_lineage_ref: registry_lineage_ref.clone(),
        initial_store_incarnation_ref: canonical_wallet_reference(&incarnation)
            .map_err(|_| ProviderTestError::Invalid)?,
        current_schema_record: record.clone(),
    };
    let registry_matches = registry_lineage_ref == state.registry_lineage_ref;
    let incarnation_matches = context.store_incarnation == incarnation;
    let confirmed_replay = {
        let retained = state
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        retained.issued.get(&domain_id) == Some(&expected)
            && retained
                .issuance_provider_heads
                .get(&domain_id)
                .is_some_and(|original_provider_head| {
                    provider_head == *original_provider_head
                        || provider_head == retained.provider_fence_head_ref
                })
    };
    let target_matches = state.probe_context(&context).await;
    if !allowed || !registry_matches || !incarnation_matches || !target_matches {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    }
    let _lease = if confirmed_replay {
        None
    } else {
        match state.acquire_issuance_lease(&incarnation, &provider_head) {
            Ok(lease) => Some(lease),
            Err(_) => {
                channel.send(&ProviderReply::Rejected).await?;
                return Ok(());
            }
        }
    };
    let mutation = ProviderMutation::ActivationIssuance {
        record: record.clone(),
        incarnation: incarnation.clone(),
        attestation: expected.clone(),
    };
    let prepared = if confirmed_replay {
        None
    } else {
        match state.prepare_checkpoint(&mutation, None).await {
            Ok(prepared) => Some(prepared),
            Err(_) => {
                let reply = if registry_domain_occupied(&state, &domain_id).await? {
                    ProviderReply::Conflict
                } else {
                    ProviderReply::Rejected
                };
                channel.send(&reply).await?;
                return Ok(());
            }
        }
    };
    let signature = channel.assertion_signature(
        "prepare-issuance",
        &(
            &record,
            &incarnation,
            &context,
            &registry_lineage_ref,
            &provider_head,
            &issuance_ref,
        ),
    )?;
    if channel
        .send(&ProviderReply::IssuancePrepared {
            registry_issuance_ref: issuance_ref.clone(),
            provider_attestation: signature,
        })
        .await
        .is_err()
    {
        if let Some(prepared) = prepared {
            let _ = state.abort_checkpoint(prepared).await;
        }
        return Ok(());
    }
    let ProviderRequest::ConfirmIssuance { attestation } = channel.receive().await? else {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    };
    if attestation != expected || !registry_row_matches(&state, &attestation).await? {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    }
    if let Some(prepared) = prepared {
        if state.acknowledge_checkpoint(prepared).await.is_err() {
            channel.send(&ProviderReply::EntryUnknown).await?;
            return Ok(());
        }
    }
    let issuance_confirmed = {
        let mut retained = state
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        match retained.issued.get(&domain_id) {
            Some(existing) => existing == &attestation,
            None => {
                retained
                    .issued
                    .insert(domain_id.clone(), attestation.clone());
                retained
                    .issuance_provider_heads
                    .insert(domain_id, provider_head);
                true
            }
        }
    };
    if !issuance_confirmed {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    }
    let signature = channel.assertion_signature(
        "confirm-issuance",
        &(&record, &incarnation, &context, &attestation),
    )?;
    channel
        .send(&ProviderReply::Confirmed {
            provider_attestation: signature,
        })
        .await
}

#[allow(clippy::too_many_arguments)]
async fn handle_promotion(
    channel: &mut ServerChannel,
    state: Arc<ProviderState>,
    successor: WalletNonceStoreSuccessor,
    context: ProviderTargetContext,
    registry_lineage_ref: EvmWalletReference,
    provider_head: EvmWalletReference,
) -> Result<(), ProviderTestError> {
    let successor_ref =
        canonical_wallet_reference(&successor).map_err(|_| ProviderTestError::Invalid)?;
    let successor_digest = successor_ref.content_digest().to_owned();
    let Some(allowed) = state.allowed_promotions.get(&successor_digest).cloned() else {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    };
    let current_prefix_digest =
        wallet_prefix_digest(&state.pool, &state.nonce_pool, &state.schema_name).await?;
    let (ready, confirmed_replay) = {
        let retained = state
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        let current_incarnation_ref = canonical_wallet_reference(&retained.current_incarnation)
            .map_err(|_| ProviderTestError::Invalid)?;
        let first_publication = retained.promotion_authorized
            && !retained.accepting
            && retained.active_leases == 0
            && retained.captured_prefix_digest.as_ref() == Some(&current_prefix_digest)
            && retained.provider_fence_head_ref == provider_head
            && current_incarnation_ref == successor.expected_current_incarnation_ref;
        let exact_replay = retained
            .confirmed_promotions
            .get(&successor_digest)
            .is_some_and(|original_provider_head| {
                provider_head == *original_provider_head
                    || provider_head == retained.provider_fence_head_ref
            })
            && retained.current_incarnation == successor.next_incarnation
            && retained.current_public_lineage_head == allowed.next_public_lineage_head;
        (first_publication || exact_replay, exact_replay)
    };
    if !ready
        || registry_lineage_ref != state.registry_lineage_ref
        || context.store_incarnation != successor.next_incarnation
        || !state.probe_context(&context).await
    {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    }
    let expected_attestation = WalletNonceStorePromotionAttestation {
        wallet_nonce_store_lineage_id: successor.wallet_nonce_store_lineage_id.clone(),
        previous_incarnation_ref: successor.expected_current_incarnation_ref.clone(),
        current_incarnation_ref: canonical_wallet_reference(&successor.next_incarnation)
            .map_err(|_| ProviderTestError::Invalid)?,
        writer_epoch: successor.next_incarnation.writer_epoch,
        successor_ref: successor_ref.clone(),
        qualified_activation_registry_lineage_ref: registry_lineage_ref.clone(),
    };
    expected_attestation
        .validate()
        .map_err(|_| ProviderTestError::Invalid)?;
    let successor_target = CheckpointTarget {
        database_oid: state.database_oid,
        schema_name: state.schema_name.clone(),
        store_incarnation: successor.next_incarnation.clone(),
        current_public_lineage_head: allowed.next_public_lineage_head.clone(),
        provider_fence_head_ref: allowed.next_provider_fence_head_ref.clone(),
    };
    let prepared = if confirmed_replay {
        None
    } else {
        let mutation = ProviderMutation::Promotion {
            successor: successor.clone(),
            attestation: expected_attestation.clone(),
        };
        match state
            .prepare_checkpoint(&mutation, Some(successor_target))
            .await
        {
            Ok(prepared) => Some(prepared),
            Err(_) => {
                channel.send(&ProviderReply::Rejected).await?;
                return Ok(());
            }
        }
    };
    let signature = channel.assertion_signature(
        "prepare-promotion",
        &(&successor, &context, &registry_lineage_ref, &provider_head),
    )?;
    if channel
        .send(&ProviderReply::PromotionPrepared {
            provider_attestation: signature,
        })
        .await
        .is_err()
    {
        if let Some(prepared) = prepared {
            let _ = state.abort_checkpoint(prepared).await;
        }
        return Ok(());
    }
    let ProviderRequest::ConfirmPromotion { attestation } = channel.receive().await? else {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    };
    let decoded: WalletNonceStorePromotionAttestation =
        serde_json::from_value(attestation.clone()).map_err(|_| ProviderTestError::Invalid)?;
    if decoded != expected_attestation
        || !promotion_row_matches(&state, &successor_ref, &decoded).await?
    {
        channel.send(&ProviderReply::Rejected).await?;
        return Ok(());
    }
    if state.promotion_crash_point == Some(ProviderPromotionCrashPoint::AfterCasBeforeOpen) {
        std::process::exit(86);
    }
    if let Some(prepared) = prepared {
        if state.acknowledge_checkpoint(prepared).await.is_err() {
            channel.send(&ProviderReply::EntryUnknown).await?;
            return Ok(());
        }
    }
    {
        let mut retained = state
            .mutable
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        if !retained
            .confirmed_promotions
            .contains_key(&successor_digest)
        {
            retained.current_incarnation = successor.next_incarnation.clone();
            retained.current_public_lineage_head = allowed.next_public_lineage_head;
            retained.provider_fence_head_ref = allowed.next_provider_fence_head_ref;
            retained
                .confirmed_promotions
                .insert(successor_digest, provider_head);
        }
        retained.accepting = true;
        retained.promotion_authorized = false;
        retained.captured_prefix_digest = None;
    }
    if state.promotion_crash_point == Some(ProviderPromotionCrashPoint::AfterOpenBeforeConfirmation)
    {
        std::process::exit(87);
    }
    let signature =
        channel.assertion_signature("confirm-promotion", &(&successor, &context, &attestation))?;
    channel
        .send(&ProviderReply::Confirmed {
            provider_attestation: signature,
        })
        .await
}

async fn wallet_prefix_digest(
    activation_pool: &PgPool,
    nonce_pool: &PgPool,
    schema_name: &str,
) -> Result<String, ProviderTestError> {
    wallet_checkpoint_prefix(activation_pool, nonce_pool, schema_name)
        .await?
        .digest()
}

async fn current_database_oid(pool: &PgPool) -> Result<u32, ProviderTestError> {
    let database_oid = sqlx::query_scalar::<_, i64>(
        "SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname = current_database()",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| ProviderTestError::Unavailable)?;
    u32::try_from(database_oid).map_err(|_| ProviderTestError::Invalid)
}

async fn wallet_checkpoint_prefix(
    activation_pool: &PgPool,
    nonce_pool: &PgPool,
    schema_name: &str,
) -> Result<WalletCheckpointPrefix, ProviderTestError> {
    let schema = quoted_identifier(schema_name)?;
    let mut prefix = WalletCheckpointPrefix::default();

    let sql = format!(
        "SELECT wallet_nonce_store_lineage_id, writer_epoch::text AS writer_epoch, \
                incarnation_ref, predecessor_incarnation_ref, incarnation_json, \
                promotion_successor_ref, promotion_attestation_json \
           FROM {schema}.wallet_store_incarnations"
    );
    for row in sqlx::query(AssertSqlSafe(sql))
        .fetch_all(activation_pool)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?
    {
        let value = StoreIncarnationCheckpointRow {
            wallet_nonce_store_lineage_id: checkpoint_column(
                &row,
                "wallet_nonce_store_lineage_id",
            )?,
            writer_epoch: checkpoint_column(&row, "writer_epoch")?,
            incarnation_ref: checkpoint_column(&row, "incarnation_ref")?,
            predecessor_incarnation_ref: checkpoint_optional_column(
                &row,
                "predecessor_incarnation_ref",
            )?,
            incarnation_json: checkpoint_column(&row, "incarnation_json")?,
            promotion_successor_ref: checkpoint_optional_column(&row, "promotion_successor_ref")?,
            promotion_attestation_json: checkpoint_optional_column(
                &row,
                "promotion_attestation_json",
            )?,
        };
        insert_checkpoint_row(
            &mut prefix.store_incarnations,
            format!(
                "{}\0{}",
                value.wallet_nonce_store_lineage_id, value.writer_epoch
            ),
            value,
        )?;
    }

    let sql = format!(
        "SELECT wallet_nonce_store_lineage_id, current_writer_epoch::text AS current_writer_epoch, \
                current_incarnation_ref FROM {schema}.wallet_store_lineage_heads"
    );
    for row in sqlx::query(AssertSqlSafe(sql))
        .fetch_all(activation_pool)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?
    {
        let value = StoreLineageHeadCheckpointRow {
            wallet_nonce_store_lineage_id: checkpoint_column(
                &row,
                "wallet_nonce_store_lineage_id",
            )?,
            current_writer_epoch: checkpoint_column(&row, "current_writer_epoch")?,
            current_incarnation_ref: checkpoint_column(&row, "current_incarnation_ref")?,
        };
        insert_checkpoint_row(
            &mut prefix.store_lineage_heads,
            value.wallet_nonce_store_lineage_id.clone(),
            value,
        )?;
    }

    let sql = format!(
        "SELECT wallet_nonce_domain_id, activation_record_ref, wallet_nonce_store_lineage_id, \
                observed_store_incarnation_ref, activation_record_json, registry_issuance_ref, \
                activation_attestation_json FROM {schema}.wallet_domain_activations"
    );
    for row in sqlx::query(AssertSqlSafe(sql))
        .fetch_all(activation_pool)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?
    {
        let value = ActivationCheckpointRow {
            wallet_nonce_domain_id: checkpoint_column(&row, "wallet_nonce_domain_id")?,
            activation_record_ref: checkpoint_column(&row, "activation_record_ref")?,
            wallet_nonce_store_lineage_id: checkpoint_column(
                &row,
                "wallet_nonce_store_lineage_id",
            )?,
            observed_store_incarnation_ref: checkpoint_column(
                &row,
                "observed_store_incarnation_ref",
            )?,
            activation_record_json: checkpoint_column(&row, "activation_record_json")?,
            registry_issuance_ref: checkpoint_column(&row, "registry_issuance_ref")?,
            activation_attestation_json: checkpoint_column(&row, "activation_attestation_json")?,
        };
        insert_checkpoint_row(
            &mut prefix.domain_activations,
            value.wallet_nonce_domain_id.clone(),
            value,
        )?;
    }

    let sql = format!(
        "SELECT wallet_nonce_domain_id, activation_record_ref, wallet_nonce_store_lineage_id, \
                activation_record_json, activation_attestation_json, \
                local_high_water_nonce::text AS local_high_water_nonce \
           FROM {schema}.wallet_nonce_domains"
    );
    for row in sqlx::query(AssertSqlSafe(sql))
        .fetch_all(nonce_pool)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?
    {
        let value = DomainCheckpointRow {
            wallet_nonce_domain_id: checkpoint_column(&row, "wallet_nonce_domain_id")?,
            activation_record_ref: checkpoint_column(&row, "activation_record_ref")?,
            wallet_nonce_store_lineage_id: checkpoint_column(
                &row,
                "wallet_nonce_store_lineage_id",
            )?,
            activation_record_json: checkpoint_column(&row, "activation_record_json")?,
            activation_attestation_json: checkpoint_column(&row, "activation_attestation_json")?,
            local_high_water_nonce: checkpoint_optional_column(&row, "local_high_water_nonce")?,
        };
        insert_checkpoint_row(
            &mut prefix.domains,
            value.wallet_nonce_domain_id.clone(),
            value,
        )?;
    }

    let sql = format!(
        "SELECT semantic_reservation_key, wallet_nonce_domain_id, submission_intent_id, \
                nonce::text AS nonce, transaction_intent_digest, candidate_family_ref, \
                request_json, transaction_intent_json, candidate_family_json, reservation_json, \
                state_input_json FROM {schema}.wallet_nonce_reservations"
    );
    for row in sqlx::query(AssertSqlSafe(sql))
        .fetch_all(nonce_pool)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?
    {
        let value = ReservationCheckpointRow {
            semantic_reservation_key: checkpoint_column(&row, "semantic_reservation_key")?,
            wallet_nonce_domain_id: checkpoint_column(&row, "wallet_nonce_domain_id")?,
            submission_intent_id: checkpoint_column(&row, "submission_intent_id")?,
            nonce: checkpoint_column(&row, "nonce")?,
            transaction_intent_digest: checkpoint_column(&row, "transaction_intent_digest")?,
            candidate_family_ref: checkpoint_column(&row, "candidate_family_ref")?,
            request_json: checkpoint_column(&row, "request_json")?,
            transaction_intent_json: checkpoint_column(&row, "transaction_intent_json")?,
            candidate_family_json: checkpoint_column(&row, "candidate_family_json")?,
            reservation_json: checkpoint_column(&row, "reservation_json")?,
            state_input_json: checkpoint_column(&row, "state_input_json")?,
        };
        insert_checkpoint_row(
            &mut prefix.reservations,
            value.semantic_reservation_key.clone(),
            value,
        )?;
    }

    let sql = format!(
        "SELECT semantic_candidate_operation_key, semantic_reservation_key, candidate_ordinal, \
                request_json, active_candidate_json, state_input_json \
           FROM {schema}.wallet_nonce_candidates"
    );
    for row in sqlx::query(AssertSqlSafe(sql))
        .fetch_all(nonce_pool)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?
    {
        let value = CandidateCheckpointRow {
            semantic_candidate_operation_key: checkpoint_column(
                &row,
                "semantic_candidate_operation_key",
            )?,
            semantic_reservation_key: checkpoint_column(&row, "semantic_reservation_key")?,
            candidate_ordinal: row
                .try_get("candidate_ordinal")
                .map_err(|_| ProviderTestError::Invalid)?,
            request_json: checkpoint_column(&row, "request_json")?,
            active_candidate_json: checkpoint_column(&row, "active_candidate_json")?,
            state_input_json: checkpoint_column(&row, "state_input_json")?,
        };
        insert_checkpoint_row(
            &mut prefix.candidates,
            value.semantic_candidate_operation_key.clone(),
            value,
        )?;
    }

    let sql = format!(
        "SELECT semantic_completion_key, semantic_reservation_key, request_json, \
                canonical_terminal_outcome_json, completion_json, state_input_json \
           FROM {schema}.wallet_nonce_completions"
    );
    for row in sqlx::query(AssertSqlSafe(sql))
        .fetch_all(nonce_pool)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?
    {
        let value = CompletionCheckpointRow {
            semantic_completion_key: checkpoint_column(&row, "semantic_completion_key")?,
            semantic_reservation_key: checkpoint_column(&row, "semantic_reservation_key")?,
            request_json: checkpoint_column(&row, "request_json")?,
            canonical_terminal_outcome_json: checkpoint_column(
                &row,
                "canonical_terminal_outcome_json",
            )?,
            completion_json: checkpoint_column(&row, "completion_json")?,
            state_input_json: checkpoint_column(&row, "state_input_json")?,
        };
        insert_checkpoint_row(
            &mut prefix.completions,
            value.semantic_completion_key.clone(),
            value,
        )?;
    }
    Ok(prefix)
}

fn checkpoint_column(row: &PgRow, column: &str) -> Result<String, ProviderTestError> {
    row.try_get(column).map_err(|_| ProviderTestError::Invalid)
}

fn checkpoint_optional_column(
    row: &PgRow,
    column: &str,
) -> Result<Option<String>, ProviderTestError> {
    row.try_get(column).map_err(|_| ProviderTestError::Invalid)
}

fn insert_checkpoint_row<T>(
    rows: &mut BTreeMap<String, T>,
    key: String,
    value: T,
) -> Result<(), ProviderTestError> {
    if rows.insert(key, value).is_some() {
        return Err(ProviderTestError::Invalid);
    }
    Ok(())
}

fn apply_checkpoint_mutation(
    prefix: &mut WalletCheckpointPrefix,
    mutation: &ProviderMutation,
) -> Result<(), ProviderTestError> {
    match mutation {
        ProviderMutation::ActivationIssuance {
            record,
            incarnation,
            attestation,
        } => {
            record.validate().map_err(|_| ProviderTestError::Invalid)?;
            incarnation
                .validate()
                .map_err(|_| ProviderTestError::Invalid)?;
            attestation
                .validate()
                .map_err(|_| ProviderTestError::Invalid)?;
            let record_ref =
                canonical_wallet_reference(record).map_err(|_| ProviderTestError::Invalid)?;
            let incarnation_ref =
                canonical_wallet_reference(incarnation).map_err(|_| ProviderTestError::Invalid)?;
            if attestation.current_schema_record != *record
                || attestation.activation_record_ref != record_ref
                || attestation.initial_store_incarnation_ref != incarnation_ref
                || record.initial_store_incarnation_ref != incarnation_ref
            {
                return Err(ProviderTestError::Invalid);
            }
            let lineage = record.wallet_nonce_store_lineage_id.clone();
            let epoch = incarnation.writer_epoch.to_string();
            let incarnation_ref_json = canonical_json(&incarnation_ref)?;
            match prefix.store_lineage_heads.get(&lineage) {
                Some(head)
                    if head.current_writer_epoch == epoch
                        && head.current_incarnation_ref == incarnation_ref_json =>
                {
                    let key = format!("{lineage}\0{epoch}");
                    let retained = prefix
                        .store_incarnations
                        .get(&key)
                        .ok_or(ProviderTestError::Invalid)?;
                    if retained.incarnation_json != canonical_json(incarnation)?
                        || retained.incarnation_ref != incarnation_ref_json
                    {
                        return Err(ProviderTestError::Invalid);
                    }
                }
                None if incarnation.writer_epoch == 1 => {
                    insert_checkpoint_row(
                        &mut prefix.store_incarnations,
                        format!("{lineage}\0{epoch}"),
                        StoreIncarnationCheckpointRow {
                            wallet_nonce_store_lineage_id: lineage.clone(),
                            writer_epoch: epoch.clone(),
                            incarnation_ref: incarnation_ref_json.clone(),
                            predecessor_incarnation_ref: None,
                            incarnation_json: canonical_json(incarnation)?,
                            promotion_successor_ref: None,
                            promotion_attestation_json: None,
                        },
                    )?;
                    insert_checkpoint_row(
                        &mut prefix.store_lineage_heads,
                        lineage.clone(),
                        StoreLineageHeadCheckpointRow {
                            wallet_nonce_store_lineage_id: lineage,
                            current_writer_epoch: epoch,
                            current_incarnation_ref: incarnation_ref_json,
                        },
                    )?;
                }
                _ => return Err(ProviderTestError::Invalid),
            }
            insert_checkpoint_row(
                &mut prefix.domain_activations,
                record.wallet_nonce_domain.as_str().to_owned(),
                ActivationCheckpointRow {
                    wallet_nonce_domain_id: record.wallet_nonce_domain.as_str().to_owned(),
                    activation_record_ref: canonical_json(&record_ref)?,
                    wallet_nonce_store_lineage_id: record.wallet_nonce_store_lineage_id.clone(),
                    observed_store_incarnation_ref: canonical_json(&incarnation_ref)?,
                    activation_record_json: canonical_json(record)?,
                    registry_issuance_ref: canonical_json(&attestation.registry_issuance_ref)?,
                    activation_attestation_json: canonical_json(attestation)?,
                },
            )
        }
        ProviderMutation::Reservation {
            request,
            reservation,
            state_input_ref,
        } => {
            if reservation.nonce_domain != request.nonce_domain
                || reservation.semantic_reservation_key != request.reservation_key
                || reservation.submission_intent_id != request.submission_intent_id
                || reservation.transaction_intent_digest != request.transaction_intent.digest()
                || reservation.candidate_family_ref != request.candidate_family.digest()
            {
                return Err(ProviderTestError::Invalid);
            }
            let domain_id = request.nonce_domain.as_str().to_owned();
            let activation = &request.domain_activation_attestation;
            let expected_domain = DomainCheckpointRow {
                wallet_nonce_domain_id: domain_id.clone(),
                activation_record_ref: canonical_json(&activation.activation_record_ref)?,
                wallet_nonce_store_lineage_id: activation
                    .current_schema_record
                    .wallet_nonce_store_lineage_id
                    .clone(),
                activation_record_json: canonical_json(&activation.current_schema_record)?,
                activation_attestation_json: canonical_json(activation)?,
                local_high_water_nonce: Some(reservation.nonce.to_string()),
            };
            if let Some(retained) = prefix.domains.get_mut(&domain_id) {
                let mut expected_predecessor = expected_domain.clone();
                expected_predecessor.local_high_water_nonce =
                    retained.local_high_water_nonce.clone();
                if *retained != expected_predecessor {
                    return Err(ProviderTestError::Invalid);
                }
                retained.local_high_water_nonce = expected_domain.local_high_water_nonce;
            } else {
                prefix.domains.insert(domain_id.clone(), expected_domain);
            }
            insert_checkpoint_row(
                &mut prefix.reservations,
                request.reservation_key.as_str().to_owned(),
                ReservationCheckpointRow {
                    semantic_reservation_key: request.reservation_key.as_str().to_owned(),
                    wallet_nonce_domain_id: domain_id,
                    submission_intent_id: request.submission_intent_id.as_str().to_owned(),
                    nonce: reservation.nonce.to_string(),
                    transaction_intent_digest: request.transaction_intent.digest().to_owned(),
                    candidate_family_ref: request.candidate_family.digest().to_owned(),
                    request_json: canonical_json(request)?,
                    transaction_intent_json: canonical_json(&request.transaction_intent)?,
                    candidate_family_json: canonical_json(&request.candidate_family)?,
                    reservation_json: canonical_json(reservation)?,
                    state_input_json: canonical_json(state_input_ref)?,
                },
            )
        }
        ProviderMutation::CandidateActivation {
            request,
            candidate,
            state_input_ref,
        } => insert_checkpoint_row(
            &mut prefix.candidates,
            request.candidate_operation_key.as_str().to_owned(),
            CandidateCheckpointRow {
                semantic_candidate_operation_key: request
                    .candidate_operation_key
                    .as_str()
                    .to_owned(),
                semantic_reservation_key: request
                    .next_candidate
                    .semantic_reservation_key
                    .as_str()
                    .to_owned(),
                candidate_ordinal: i32::from(request.next_candidate.candidate_ordinal),
                request_json: canonical_json(request)?,
                active_candidate_json: canonical_json(candidate)?,
                state_input_json: canonical_json(state_input_ref)?,
            },
        ),
        ProviderMutation::Completion {
            request,
            completion,
            state_input_ref,
        } => insert_checkpoint_row(
            &mut prefix.completions,
            request.completion_key.as_str().to_owned(),
            CompletionCheckpointRow {
                semantic_completion_key: request.completion_key.as_str().to_owned(),
                semantic_reservation_key: request
                    .current_reservation
                    .semantic_reservation_key
                    .as_str()
                    .to_owned(),
                request_json: canonical_json(request)?,
                canonical_terminal_outcome_json: canonical_json(
                    &request.canonical_terminal_outcome,
                )?,
                completion_json: canonical_json(completion)?,
                state_input_json: canonical_json(state_input_ref)?,
            },
        ),
        ProviderMutation::Promotion {
            successor,
            attestation,
        } => {
            successor
                .validate()
                .map_err(|_| ProviderTestError::Invalid)?;
            attestation
                .validate()
                .map_err(|_| ProviderTestError::Invalid)?;
            let successor_ref =
                canonical_wallet_reference(successor).map_err(|_| ProviderTestError::Invalid)?;
            let next_ref = canonical_wallet_reference(&successor.next_incarnation)
                .map_err(|_| ProviderTestError::Invalid)?;
            if attestation.successor_ref != successor_ref
                || attestation.previous_incarnation_ref
                    != successor.expected_current_incarnation_ref
                || attestation.current_incarnation_ref != next_ref
                || attestation.writer_epoch != successor.next_incarnation.writer_epoch
            {
                return Err(ProviderTestError::Invalid);
            }
            let lineage = successor.wallet_nonce_store_lineage_id.clone();
            let head = prefix
                .store_lineage_heads
                .get_mut(&lineage)
                .ok_or(ProviderTestError::Invalid)?;
            if head.current_incarnation_ref
                != canonical_json(&successor.expected_current_incarnation_ref)?
                || head
                    .current_writer_epoch
                    .parse::<u64>()
                    .ok()
                    .and_then(|value| value.checked_add(1))
                    != Some(successor.next_incarnation.writer_epoch)
            {
                return Err(ProviderTestError::Invalid);
            }
            let epoch = successor.next_incarnation.writer_epoch.to_string();
            let next_ref_json = canonical_json(&next_ref)?;
            insert_checkpoint_row(
                &mut prefix.store_incarnations,
                format!("{lineage}\0{epoch}"),
                StoreIncarnationCheckpointRow {
                    wallet_nonce_store_lineage_id: lineage.clone(),
                    writer_epoch: epoch.clone(),
                    incarnation_ref: next_ref_json.clone(),
                    predecessor_incarnation_ref: Some(canonical_json(
                        &successor.expected_current_incarnation_ref,
                    )?),
                    incarnation_json: canonical_json(&successor.next_incarnation)?,
                    promotion_successor_ref: Some(canonical_json(&successor_ref)?),
                    promotion_attestation_json: Some(canonical_json(attestation)?),
                },
            )?;
            head.current_writer_epoch = epoch;
            head.current_incarnation_ref = next_ref_json;
            Ok(())
        }
    }
}

fn checkpoint_mutation_with_provider_attestation(
    mutation: &ProviderMutation,
    provider_attestation: &str,
) -> Result<ProviderMutation, ProviderTestError> {
    if provider_attestation.is_empty()
        || provider_attestation.len() > 4_096
        || !provider_attestation
            .bytes()
            .all(|byte| byte.is_ascii_graphic())
    {
        return Err(ProviderTestError::Invalid);
    }
    match mutation {
        ProviderMutation::CandidateActivation {
            request,
            candidate,
            state_input_ref,
        } => {
            let mut candidate = candidate.clone();
            if !candidate.provider_activation_attestation.is_empty() {
                return Err(ProviderTestError::Invalid);
            }
            candidate.provider_activation_attestation = provider_attestation.to_owned();
            Ok(ProviderMutation::CandidateActivation {
                request: request.clone(),
                candidate,
                state_input_ref: state_input_ref.clone(),
            })
        }
        ProviderMutation::Completion {
            request,
            completion,
            state_input_ref,
        } => {
            let completion = completion
                .clone()
                .with_provider_completion_attestation(provider_attestation.to_owned())
                .map_err(|_| ProviderTestError::Invalid)?;
            Ok(ProviderMutation::Completion {
                request: request.clone(),
                completion,
                state_input_ref: state_input_ref.clone(),
            })
        }
        ProviderMutation::ActivationIssuance { .. }
        | ProviderMutation::Reservation { .. }
        | ProviderMutation::Promotion { .. } => Ok(mutation.clone()),
    }
}

fn issued_reference_matches(
    state: &ProviderState,
    issuance_ref: &EvmWalletReference,
    incarnation: &WalletNonceStoreIncarnation,
    provider_head: &EvmWalletReference,
) -> Result<bool, ProviderTestError> {
    let retained = state
        .mutable
        .lock()
        .map_err(|_| ProviderTestError::Unavailable)?;
    Ok(provider_head.to_content_ref().is_ok()
        && retained.issued.values().any(|attestation| {
            &attestation.registry_issuance_ref == issuance_ref
                && attestation
                    .current_schema_record
                    .wallet_nonce_store_lineage_id
                    == incarnation.wallet_nonce_store_lineage_id
        }))
}

async fn send_superseded_or_unavailable(
    channel: &mut ServerChannel,
    state: &ProviderState,
) -> Result<(), ProviderTestError> {
    if let Some(supersession) = &state.supersession {
        channel
            .send(&ProviderReply::Superseded {
                evidence: supersession.evidence.clone(),
                public_head: supersession.public_head.clone(),
            })
            .await
    } else {
        channel.send(&ProviderReply::Unavailable).await
    }
}

async fn registry_row_matches(
    state: &ProviderState,
    attestation: &WalletNonceDomainActivationAttestation,
) -> Result<bool, ProviderTestError> {
    let sql = format!(
        "SELECT wallet_nonce_domain_id, activation_record_ref, \
                wallet_nonce_store_lineage_id, observed_store_incarnation_ref, \
                activation_record_json, registry_issuance_ref, activation_attestation_json \
           FROM {}.wallet_domain_activations \
         WHERE wallet_nonce_domain_id = $1",
        quoted_identifier(&state.schema_name)?
    );
    let domain = attestation
        .current_schema_record
        .wallet_nonce_domain
        .as_str();
    let retained = sqlx::query(AssertSqlSafe(sql))
        .bind(domain)
        .fetch_optional(&state.pool)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?;
    Ok(retained
        .as_ref()
        .map(|row| decode_retained_activation(row, Some(domain)))
        .transpose()?
        .as_ref()
        == Some(attestation))
}

async fn registry_domain_occupied(
    state: &ProviderState,
    domain_id: &str,
) -> Result<bool, ProviderTestError> {
    let sql = format!(
        "SELECT EXISTS (SELECT 1 FROM {}.wallet_domain_activations \
         WHERE wallet_nonce_domain_id = $1)",
        quoted_identifier(&state.schema_name)?
    );
    sqlx::query_scalar::<_, bool>(AssertSqlSafe(sql))
        .bind(domain_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| ProviderTestError::Unavailable)
}

fn decode_retained_activation(
    row: &PgRow,
    expected_domain: Option<&str>,
) -> Result<WalletNonceDomainActivationAttestation, ProviderTestError> {
    let domain = row
        .try_get::<String, _>("wallet_nonce_domain_id")
        .map_err(|_| ProviderTestError::Invalid)?;
    let activation_record_ref = row
        .try_get::<String, _>("activation_record_ref")
        .map_err(|_| ProviderTestError::Invalid)?;
    let lineage_id = row
        .try_get::<String, _>("wallet_nonce_store_lineage_id")
        .map_err(|_| ProviderTestError::Invalid)?;
    let observed_incarnation_ref = row
        .try_get::<String, _>("observed_store_incarnation_ref")
        .map_err(|_| ProviderTestError::Invalid)?;
    let record_json = row
        .try_get::<String, _>("activation_record_json")
        .map_err(|_| ProviderTestError::Invalid)?;
    let registry_issuance_ref = row
        .try_get::<String, _>("registry_issuance_ref")
        .map_err(|_| ProviderTestError::Invalid)?;
    let attestation_json = row
        .try_get::<String, _>("activation_attestation_json")
        .map_err(|_| ProviderTestError::Invalid)?;
    let record: WalletNonceDomainActivationRecord =
        serde_json::from_str(&record_json).map_err(|_| ProviderTestError::Invalid)?;
    let attestation: WalletNonceDomainActivationAttestation =
        serde_json::from_str(&attestation_json).map_err(|_| ProviderTestError::Invalid)?;
    record.validate().map_err(|_| ProviderTestError::Invalid)?;
    attestation
        .validate()
        .map_err(|_| ProviderTestError::Invalid)?;
    if expected_domain.is_some_and(|expected| expected != domain)
        || domain != record.wallet_nonce_domain.as_str()
        || lineage_id != record.wallet_nonce_store_lineage_id
        || canonical_json(&record)? != record_json
        || attestation.current_schema_record != record
        || canonical_json(&attestation)? != attestation_json
        || canonical_json(&attestation.activation_record_ref)? != activation_record_ref
        || canonical_json(&attestation.initial_store_incarnation_ref)? != observed_incarnation_ref
        || canonical_json(&attestation.registry_issuance_ref)? != registry_issuance_ref
    {
        return Err(ProviderTestError::Invalid);
    }
    Ok(attestation)
}

async fn promotion_row_matches(
    state: &ProviderState,
    successor_ref: &EvmWalletReference,
    attestation: &WalletNonceStorePromotionAttestation,
) -> Result<bool, ProviderTestError> {
    let sql = format!(
        "SELECT promotion_attestation_json FROM {}.wallet_store_incarnations \
         WHERE promotion_successor_ref = $1",
        quoted_identifier(&state.schema_name)?
    );
    let reference = canonical_json(successor_ref)?;
    let retained = sqlx::query_scalar::<_, String>(AssertSqlSafe(sql))
        .bind(reference)
        .fetch_optional(&state.pool)
        .await
        .map_err(|_| ProviderTestError::Unavailable)?;
    let expected = canonical_json(attestation)?;
    Ok(retained.as_deref() == Some(expected.as_str()))
}

fn quoted_identifier(value: &str) -> Result<String, ProviderTestError> {
    if value.is_empty()
        || value.len() > 63
        || !value.chars().enumerate().all(|(index, character)| {
            character == '_'
                || (character.is_ascii_alphanumeric() && (index > 0 || !character.is_ascii_digit()))
        })
    {
        return Err(ProviderTestError::Invalid);
    }
    Ok(format!("\"{value}\""))
}

fn provider_issuance_reference(
    provider_id: &str,
    record_ref: &EvmWalletReference,
) -> Result<EvmWalletReference, ProviderTestError> {
    let digest = mfm_journal::structured::domain_content_digest(
        "mfm.wallet-authority-provider.registry-issuance.v1",
        &(provider_id, record_ref),
    )
    .map_err(|_| ProviderTestError::Invalid)?;
    let schema = SchemaId::new(
        "mfm.wallet-authority-provider.registry-issuance",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(
            b"mfm.structured-schema.v1:mfm.wallet-authority-provider.registry-issuance:1",
        ),
    )
    .map_err(|_| ProviderTestError::Invalid)?;
    let reference = ContentRef::new(schema, digest).map_err(|_| ProviderTestError::Invalid)?;
    Ok(EvmWalletReference::from_content_ref(reference))
}

struct ServerChannel {
    read: BufReader<tokio::io::ReadHalf<UnixStream>>,
    write: tokio::io::WriteHalf<UnixStream>,
    authentication_challenge: [u8; 32],
    key_pair: Arc<Ed25519KeyPair>,
}

impl ServerChannel {
    async fn send<T: Serialize>(&mut self, value: &T) -> Result<(), ProviderTestError> {
        let encoded =
            Zeroizing::new(serde_json::to_vec(value).map_err(|_| ProviderTestError::Invalid)?);
        if encoded.len() > MAX_MESSAGE_BYTES || encoded.contains(&b'\n') {
            return Err(ProviderTestError::Invalid);
        }
        self.write
            .write_all(&encoded)
            .await
            .map_err(|_| ProviderTestError::Unavailable)?;
        self.write
            .write_all(b"\n")
            .await
            .map_err(|_| ProviderTestError::Unavailable)?;
        self.write
            .flush()
            .await
            .map_err(|_| ProviderTestError::Unavailable)
    }

    async fn receive<T: DeserializeOwned>(&mut self) -> Result<T, ProviderTestError> {
        let encoded = read_bounded_frame(&mut self.read, MAX_MESSAGE_BYTES)
            .await
            .map_err(|_| ProviderTestError::Unavailable)?;
        serde_json::from_slice(&encoded).map_err(|_| ProviderTestError::Invalid)
    }

    fn authentication_signature(&self, state: &ProviderState) -> Result<String, ProviderTestError> {
        let mut signed = Vec::new();
        signed.extend_from_slice(AUTHENTICATION_DOMAIN);
        signed.extend_from_slice(&self.authentication_challenge);
        signed.extend_from_slice(state.provider_id.as_bytes());
        signed.extend_from_slice(state.fence_lineage_ref.content_digest().as_bytes());
        signed.extend_from_slice(state.chain_registry_lineage_ref.content_digest().as_bytes());
        signed.extend_from_slice(
            state
                .current_chain_registry_head_ref
                .content_digest()
                .as_bytes(),
        );
        Ok(hex::encode(self.key_pair.sign(&signed).as_ref()))
    }

    fn assertion_signature<T: Serialize>(
        &self,
        kind: &str,
        payload: &T,
    ) -> Result<String, ProviderTestError> {
        let digest = mfm_journal::structured::domain_content_digest(
            "mfm.wallet-authority-provider.assertion-payload.v1",
            payload,
        )
        .map_err(|_| ProviderTestError::Invalid)?;
        let mut signed = Vec::new();
        signed.extend_from_slice(ASSERTION_DOMAIN);
        signed.extend_from_slice(&self.authentication_challenge);
        signed.extend_from_slice(kind.as_bytes());
        signed.extend_from_slice(digest.as_str().as_bytes());
        Ok(hex::encode(self.key_pair.sign(&signed).as_ref()))
    }

    fn persisted_mutation_attestation(
        &self,
        provider_id: &str,
        context: &ProviderTargetContext,
        operation_key: &str,
        mutation: &ProviderMutation,
        signature: &str,
    ) -> Result<String, ProviderTestError> {
        let payload_digest = mfm_journal::structured::domain_content_digest(
            "mfm.wallet-authority-provider.assertion-payload.v1",
            &(context, operation_key, mutation),
        )
        .map_err(|_| ProviderTestError::Invalid)?;
        let proof = PersistedMutationProof {
            provider_id: provider_id.to_owned(),
            challenge: hex::encode(self.authentication_challenge),
            context: context.clone(),
            operation_key: operation_key.to_owned(),
            payload_digest: payload_digest.as_str().to_owned(),
            signature: signature.to_owned(),
        };
        let encoded = serde_json::to_string(&proof).map_err(|_| ProviderTestError::Invalid)?;
        if encoded.len() > 4_096 || !encoded.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(ProviderTestError::Invalid);
        }
        Ok(encoded)
    }
}

fn read_bounded_sync_frame<R: BufRead>(
    reader: &mut R,
    maximum_payload_bytes: usize,
) -> std::io::Result<Zeroizing<Vec<u8>>> {
    let mut encoded = Zeroizing::new(Vec::new());
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "provider frame ended before its delimiter",
            ));
        }
        if let Some(delimiter) = available.iter().position(|byte| *byte == b'\n') {
            if encoded.len().saturating_add(delimiter) > maximum_payload_bytes {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "provider frame exceeds its byte bound",
                ));
            }
            encoded.extend_from_slice(&available[..delimiter]);
            reader.consume(delimiter + 1);
            return Ok(encoded);
        }
        if encoded.len().saturating_add(available.len()) > maximum_payload_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "provider frame exceeds its byte bound",
            ));
        }
        encoded.extend_from_slice(available);
        let consumed = available.len();
        reader.consume(consumed);
    }
}

async fn read_bounded_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    maximum_payload_bytes: usize,
) -> std::io::Result<Zeroizing<Vec<u8>>> {
    let mut encoded = Zeroizing::new(Vec::new());
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "provider frame ended before its delimiter",
            ));
        }
        if let Some(delimiter) = available.iter().position(|byte| *byte == b'\n') {
            if encoded.len().saturating_add(delimiter) > maximum_payload_bytes {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "provider frame exceeds its byte bound",
                ));
            }
            encoded.extend_from_slice(&available[..delimiter]);
            reader.consume(delimiter + 1);
            return Ok(encoded);
        }
        if encoded.len().saturating_add(available.len()) > maximum_payload_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "provider frame exceeds its byte bound",
            ));
        }
        encoded.extend_from_slice(available);
        let consumed = available.len();
        reader.consume(consumed);
    }
}

fn random_marker() -> Result<String, ProviderTestError> {
    Ok(hex::encode(random_bytes::<31>()?))
}

fn target_session_marker_lock_ids(marker: &str) -> Option<(i64, i64)> {
    if marker.len() != 62 || !marker.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let first = u32::from_str_radix(&marker[..8], 16).ok()?;
    let second = u32::from_str_radix(&marker[8..16], 16).ok()?;
    Some((i64::from(first), i64::from(second)))
}

fn random_bytes<const N: usize>() -> Result<Zeroizing<[u8; N]>, ProviderTestError> {
    let mut bytes = Zeroizing::new([0_u8; N]);
    SystemRandom::new()
        .fill(bytes.as_mut())
        .map_err(|_| ProviderTestError::Unavailable)?;
    Ok(bytes)
}

fn canonical_json<T: Serialize>(value: &T) -> Result<String, ProviderTestError> {
    let json = serde_json::to_string(value).map_err(|_| ProviderTestError::Invalid)?;
    mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.as_str().to_owned())
        .map_err(|_| ProviderTestError::Invalid)
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[tokio::test]
    async fn server_and_startup_frame_reader_rejects_oversized_and_unterminated_peers() {
        for payload in [
            vec![b'x'; MAX_MESSAGE_BYTES + 1],
            b"unterminated-provider-request".to_vec(),
        ] {
            let (mut peer, server) = tokio::io::duplex(payload.len() + 1);
            let writer = tokio::spawn(async move {
                peer.write_all(&payload).await.expect("hostile payload");
            });
            let mut reader = BufReader::new(server);
            let error = read_bounded_frame(&mut reader, MAX_MESSAGE_BYTES)
                .await
                .expect_err("hostile peer frame must fail");
            assert!(matches!(
                error.kind(),
                std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
            ));
            writer.await.expect("hostile writer");
        }
    }

    #[test]
    fn checkpoint_authentication_rejects_forged_shutdown_and_prepare() {
        let authentication_token = Zeroizing::new([0x41_u8; 32]);
        let target = checkpoint_target();
        let digest = checkpoint_digest(0x51);
        let state = CheckpointAuthorityState {
            authentication_token,
            fence_lineage_ref: checkpoint_reference(0x52),
            target: target.clone(),
            epoch: 0,
            acknowledged_prefix_digest: digest.clone(),
            prepared: None,
        };
        let prepared = PreparedCheckpoint {
            epoch: 1,
            predecessor_digest: digest,
            successor_digest: checkpoint_digest(0x53),
            operation_digest: checkpoint_digest(0x54),
            successor_target: None,
        };
        for request in [
            CheckpointRequest::Shutdown,
            CheckpointRequest::Prepare {
                fence_lineage_ref: state.fence_lineage_ref.clone(),
                target,
                expected_epoch: 0,
                prepared,
            },
        ] {
            let forged = authenticate_checkpoint_request(&[0x42; 32], request)
                .expect("construct forged checkpoint envelope");
            assert!(authenticate_checkpoint_envelope(&state, forged).is_err());
        }
    }

    #[test]
    fn prepared_predecessor_restart_retains_only_the_identical_successor() {
        let target = checkpoint_target();
        let predecessor = checkpoint_digest(0x61);
        let prepared = PreparedCheckpoint {
            epoch: 1,
            predecessor_digest: predecessor.clone(),
            successor_digest: checkpoint_digest(0x62),
            operation_digest: checkpoint_digest(0x63),
            successor_target: None,
        };
        let mut state = CheckpointAuthorityState {
            authentication_token: Zeroizing::new([0x64; 32]),
            fence_lineage_ref: checkpoint_reference(0x65),
            target: target.clone(),
            epoch: 0,
            acknowledged_prefix_digest: predecessor.clone(),
            prepared: Some(prepared.clone()),
        };
        assert!(matches!(
            apply_checkpoint_request(
                &mut state,
                CheckpointRequest::Verify {
                    fence_lineage_ref: checkpoint_reference(0x65),
                    target: target.clone(),
                    observed_prefix_digest: predecessor,
                },
            ),
            Ok(CheckpointReply::Current { epoch: 0 })
        ));
        assert_eq!(state.prepared, Some(prepared.clone()));
        assert!(apply_checkpoint_request(
            &mut state,
            CheckpointRequest::Prepare {
                fence_lineage_ref: checkpoint_reference(0x65),
                target: target.clone(),
                expected_epoch: 0,
                prepared: prepared.clone(),
            },
        )
        .is_ok());
        let mut competing = prepared;
        competing.successor_digest = checkpoint_digest(0x66);
        assert!(apply_checkpoint_request(
            &mut state,
            CheckpointRequest::Prepare {
                fence_lineage_ref: checkpoint_reference(0x65),
                target: target.clone(),
                expected_epoch: 0,
                prepared: competing,
            },
        )
        .is_err());
        let mut foreign_target = target;
        foreign_target.database_oid += 1;
        let acknowledged = state.acknowledged_prefix_digest.clone();
        assert!(apply_checkpoint_request(
            &mut state,
            CheckpointRequest::Verify {
                fence_lineage_ref: checkpoint_reference(0x65),
                target: foreign_target,
                observed_prefix_digest: acknowledged,
            },
        )
        .is_err());
    }

    #[test]
    fn idle_recovery_aborts_prepared_predecessor_before_a_fresh_mutation() {
        let target = checkpoint_target();
        let predecessor = checkpoint_digest(0x67);
        let prepared = PreparedCheckpoint {
            epoch: 1,
            predecessor_digest: predecessor.clone(),
            successor_digest: checkpoint_digest(0x68),
            operation_digest: checkpoint_digest(0x69),
            successor_target: None,
        };
        let mut state = CheckpointAuthorityState {
            authentication_token: Zeroizing::new([0x6a; 32]),
            fence_lineage_ref: checkpoint_reference(0x6b),
            target: target.clone(),
            epoch: 0,
            acknowledged_prefix_digest: predecessor.clone(),
            prepared: Some(prepared),
        };
        assert!(matches!(
            apply_checkpoint_request(
                &mut state,
                CheckpointRequest::Recover {
                    fence_lineage_ref: checkpoint_reference(0x6b),
                    target: target.clone(),
                    observed_prefix_digest: predecessor.clone(),
                },
            ),
            Ok(CheckpointReply::Current { epoch: 0 })
        ));
        assert!(state.prepared.is_none());

        let replacement = PreparedCheckpoint {
            epoch: 1,
            predecessor_digest: predecessor,
            successor_digest: checkpoint_digest(0x6c),
            operation_digest: checkpoint_digest(0x6d),
            successor_target: None,
        };
        assert!(apply_checkpoint_request(
            &mut state,
            CheckpointRequest::Prepare {
                fence_lineage_ref: checkpoint_reference(0x6b),
                target,
                expected_epoch: 0,
                prepared: replacement,
            },
        )
        .is_ok());
    }

    #[test]
    fn deployment_policy_rejects_every_field_substitution() {
        let reference = |byte| {
            checkpoint_reference(byte)
                .to_content_ref()
                .expect("content reference")
        };
        let digest = |byte| {
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                mfm_canonical::sha256_digest_bytes(&[byte]),
            )
        };
        let semantic_signer_id = StableId::new("mfm.test/deployment-signer").expect("signer");
        let generation = reference(0x70);
        let fence = reference(0x71);
        let exclusion = reference(0x72);
        let semantics = (0x73..0x79).map(reference).collect::<Vec<_>>();
        let releases = (0x79..0x7f).map(digest).collect::<Vec<_>>();
        let policy = ProviderDeploymentAssemblyPolicy::new(
            semantic_signer_id.clone(),
            generation.clone(),
            fence.clone(),
            exclusion.clone(),
            semantics.clone(),
            releases.clone(),
        )
        .expect("deployment policy");
        assert!(deployment_policy_fields_match(
            &policy,
            &semantic_signer_id,
            &generation,
            &fence,
            &exclusion,
            &semantics,
            &releases,
        ));
        assert!(!deployment_policy_fields_match(
            &policy,
            &StableId::new("mfm.test/foreign-signer").expect("foreign signer"),
            &generation,
            &fence,
            &exclusion,
            &semantics,
            &releases,
        ));
        assert!(!deployment_policy_fields_match(
            &policy,
            &semantic_signer_id,
            &reference(0x80),
            &fence,
            &exclusion,
            &semantics,
            &releases,
        ));
        assert!(!deployment_policy_fields_match(
            &policy,
            &semantic_signer_id,
            &generation,
            &reference(0x81),
            &exclusion,
            &semantics,
            &releases,
        ));
        assert!(!deployment_policy_fields_match(
            &policy,
            &semantic_signer_id,
            &generation,
            &fence,
            &reference(0x82),
            &semantics,
            &releases,
        ));
        let mut foreign_semantics = semantics.clone();
        foreign_semantics[3] = reference(0x83);
        assert!(!deployment_policy_fields_match(
            &policy,
            &semantic_signer_id,
            &generation,
            &fence,
            &exclusion,
            &foreign_semantics,
            &releases,
        ));
        let mut foreign_releases = releases.clone();
        foreign_releases[4] = digest(0x84);
        assert!(!deployment_policy_fields_match(
            &policy,
            &semantic_signer_id,
            &generation,
            &fence,
            &exclusion,
            &semantics,
            &foreign_releases,
        ));
    }

    #[test]
    fn boxed_mutation_keeps_provider_request_stack_bounded() {
        assert!(std::mem::size_of::<ProviderRequest>() < 64 * 1024);
    }

    #[test]
    fn deployment_lease_expiry_is_strict_and_pruned() {
        let now = Instant::now();
        assert!(deployment_lease_is_current(
            now + DEPLOYMENT_ASSEMBLY_LEASE_TTL,
            now
        ));
        assert!(!deployment_lease_is_current(now, now));
        assert!(!deployment_lease_is_current(
            now - Duration::from_nanos(1),
            now
        ));
        let mut leases = DeploymentAssemblyLeaseTable::default();
        assert!(leases.insert("expired".to_owned(), 1_u8, now));
        assert!(leases.insert("current".to_owned(), 2_u8, now + Duration::from_nanos(1)));
        leases.prune(now);
        assert!(leases.get("expired").is_none());
        assert_eq!(leases.get("current"), Some(&2));
    }

    #[test]
    fn zeroizing_finish_authorization_preserves_wire_schema() {
        let authorization = "a5".repeat(32);
        let encoded = Zeroizing::new(
            serde_json::to_vec(&ProviderReply::DeploymentAssemblyFinished {
                finish_authorization: Zeroizing::new(authorization.clone()),
                provider_attestation: "provider-attestation".to_owned(),
            })
            .expect("encode finish reply"),
        );
        let wire: serde_json::Value =
            serde_json::from_slice(&encoded).expect("decode finish reply wire schema");
        assert_eq!(wire["kind"], "deployment_assembly_finished");
        assert_eq!(wire["finish_authorization"], authorization);
        assert_eq!(wire["provider_attestation"], "provider-attestation");
    }

    fn checkpoint_target() -> CheckpointTarget {
        let object_type = StableId::new("mfm.test/checkpoint-head").expect("object type");
        let schema = SchemaId::new(
            "mfm.test.checkpoint-head",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.test.checkpoint-head.v1"),
        )
        .expect("history schema");
        CheckpointTarget {
            database_oid: 1,
            schema_name: "wallet_checkpoint_test".to_owned(),
            store_incarnation: WalletNonceStoreIncarnation {
                wallet_nonce_store_lineage_id: "mfm.test/checkpoint-lineage".to_owned(),
                writer_epoch: 1,
                physical_target_instance_id: "mfm.test/checkpoint-target".to_owned(),
                non_exportable_target_public_key_ref: checkpoint_reference(0x55),
                target_attestation_contract_ref: checkpoint_reference(0x56),
            },
            current_public_lineage_head: HistoryObject::new(object_type, schema, "{}")
                .expect("history object"),
            provider_fence_head_ref: checkpoint_reference(0x57),
        }
    }

    fn checkpoint_reference(discriminator: u8) -> EvmWalletReference {
        let schema = SchemaId::new(
            "mfm.test.checkpoint-reference",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.test.checkpoint-reference.v1"),
        )
        .expect("reference schema");
        EvmWalletReference::from_content_ref(
            ContentRef::new(
                schema,
                ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    mfm_canonical::sha256_digest_bytes(&[discriminator]),
                ),
            )
            .expect("content reference"),
        )
    }

    fn checkpoint_digest(discriminator: u8) -> String {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            mfm_canonical::sha256_digest_bytes(&[discriminator]),
        )
        .as_str()
        .to_owned()
    }
}
