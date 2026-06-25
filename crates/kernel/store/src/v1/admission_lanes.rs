use std::marker::PhantomData;

use super::*;

const ADMISSION_LANE_ID_DOMAIN: &[u8] = b"mfm.admission_lane.id.v1";
const RESOURCE_WAIT_FIFO_TOKEN_DOMAIN: &[u8] = b"mfm.admission_lane.resource.wait_fifo.token.v1";
const ADMISSION_WAITER_ID_DOMAIN: &[u8] = b"mfm.admission_lane.waiter.id.v1";
const ADMISSION_ADVISORY_LOCK_DOMAIN: &[u8] = b"mfm.admission_lane.advisory_lock.v1";

/// V1 execution-claim lease time-to-live in seconds.
pub const EXECUTION_CLAIM_LEASE_TTL_SECS: u64 = 60;

/// V1 execution-claim heartbeat interval in seconds.
pub const EXECUTION_CLAIM_HEARTBEAT_INTERVAL_SECS: u64 = 20;

/// Class of an operational admission lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AdmissionLaneClass {
    /// Side-effect resource lane admission.
    ResourceLane,
    /// Per-run execution claim admission.
    ExecutionClaim,
}

impl AdmissionLaneClass {
    /// Returns the stable string used in canonical lane material.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ResourceLane => "resource_lane",
            Self::ExecutionClaim => "execution_claim",
        }
    }
}

/// Operational admission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AdmissionLaneMode {
    /// FIFO admission for resource lanes.
    WaitFifo,
    /// Non-queueing try-admit admission for execution claims.
    NowaitSkip,
}

impl AdmissionLaneMode {
    /// Returns the stable string used in canonical lane material.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WaitFifo => "wait_fifo",
            Self::NowaitSkip => "nowait_skip",
        }
    }
}

/// Versioned admission lane id bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdmissionLaneId([u8; 32]);

impl AdmissionLaneId {
    /// Returns the raw lane id bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the lane id bytes as an owned vector.
    pub fn to_vec(self) -> Vec<u8> {
        self.0.to_vec()
    }

    fn from_material(
        class: AdmissionLaneClass,
        mode: AdmissionLaneMode,
        material: serde_json::Value,
    ) -> Result<Self> {
        let canonical = canonical_json(serde_json::json!({
            "class": class.as_str(),
            "domain": String::from_utf8_lossy(ADMISSION_LANE_ID_DOMAIN),
            "material": material,
            "mode": mode.as_str(),
        }))?;
        let mut input =
            Vec::with_capacity(ADMISSION_LANE_ID_DOMAIN.len() + canonical.as_bytes().len());
        input.extend_from_slice(ADMISSION_LANE_ID_DOMAIN);
        input.extend_from_slice(canonical.as_bytes());
        let digest = sha256_digest_bytes(&input);
        let mut lane_id = [0_u8; 32];
        lane_id[0] = 1;
        lane_id[1..].copy_from_slice(&digest.as_bytes()[..31]);
        Ok(Self(lane_id))
    }
}

/// Erased admission lane key for backend coordination primitives.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdmissionLaneKey {
    class: AdmissionLaneClass,
    id: AdmissionLaneId,
}

impl AdmissionLaneKey {
    /// Returns the lane class.
    pub fn class(&self) -> AdmissionLaneClass {
        self.class
    }

    /// Returns the lane id.
    pub fn id(&self) -> AdmissionLaneId {
        self.id
    }
}

/// Marker for FIFO admission lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitFifo;

/// Marker for non-queueing try-admit lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NowaitSkip;

mod sealed {
    pub trait Sealed {}
}

/// Sealed marker trait for admission lane modes.
pub trait AdmissionModeSpec: sealed::Sealed {
    /// Mode represented by the marker.
    const MODE: AdmissionLaneMode;
}

impl sealed::Sealed for WaitFifo {}

impl AdmissionModeSpec for WaitFifo {
    const MODE: AdmissionLaneMode = AdmissionLaneMode::WaitFifo;
}

impl sealed::Sealed for NowaitSkip {}

impl AdmissionModeSpec for NowaitSkip {
    const MODE: AdmissionLaneMode = AdmissionLaneMode::NowaitSkip;
}

/// Typed admission lane with mode fixed by `M`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionLane<M: AdmissionModeSpec> {
    class: AdmissionLaneClass,
    id: AdmissionLaneId,
    _mode: PhantomData<M>,
}

impl<M: AdmissionModeSpec> AdmissionLane<M> {
    fn new(class: AdmissionLaneClass, id: AdmissionLaneId) -> Self {
        Self {
            class,
            id,
            _mode: PhantomData,
        }
    }

    /// Returns the fixed class for this lane.
    pub fn class(&self) -> AdmissionLaneClass {
        self.class
    }

    /// Returns the fixed mode for this lane.
    pub fn mode(&self) -> AdmissionLaneMode {
        M::MODE
    }

    /// Returns the lane id.
    pub fn id(&self) -> AdmissionLaneId {
        self.id
    }

    /// Returns the erased backend key for this typed lane.
    pub fn erased_key(&self) -> AdmissionLaneKey {
        AdmissionLaneKey {
            class: self.class,
            id: self.id,
        }
    }
}

/// Resource-lane admission key. Resource lanes always use FIFO admission.
pub type ResourceAdmissionLane = AdmissionLane<WaitFifo>;

impl ResourceAdmissionLane {
    /// Builds the FIFO admission lane for resource-key evidence.
    pub fn from_resource_key_evidence(evidence: &events::ResourceKeyEvidence) -> Result<Self> {
        let key_canonical_json = resource_key_canonical_json(evidence)?;
        let material = serde_json::json!({
            "key_canonical_json": String::from_utf8(key_canonical_json).map_err(|error| {
                StoreError::Canonical(format!("resource key canonical JSON was not UTF-8: {error}"))
            })?,
            "key_schema_id": evidence.key_schema_id.as_str(),
            "namespace": evidence.namespace.as_str(),
        });
        Ok(Self::new(
            AdmissionLaneClass::ResourceLane,
            AdmissionLaneId::from_material(
                AdmissionLaneClass::ResourceLane,
                AdmissionLaneMode::WaitFifo,
                material,
            )?,
        ))
    }

    /// Builds the FIFO admission lane for an already-normalized resource lane key.
    pub fn from_resource_lane_key(key: &ResourceLaneKey) -> Result<Self> {
        Self::from_resource_key_evidence(&events::ResourceKeyEvidence {
            namespace: key.namespace.clone(),
            key_schema_id: key.key_schema_id.clone(),
            key: key.key.clone(),
        })
    }
}

/// Execution-claim admission key. Execution claims always use nowait admission.
pub type ExecutionClaimAdmissionLane = AdmissionLane<NowaitSkip>;

impl ExecutionClaimAdmissionLane {
    /// Builds the nowait admission lane for a derived run id.
    pub fn from_run_id(run_id: &RunId) -> Result<Self> {
        Ok(Self::new(
            AdmissionLaneClass::ExecutionClaim,
            AdmissionLaneId::from_material(
                AdmissionLaneClass::ExecutionClaim,
                AdmissionLaneMode::NowaitSkip,
                serde_json::json!({
                    "run_id": run_id.as_str(),
                }),
            )?,
        ))
    }
}

/// Operational admission token.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdmissionToken(String);

impl AdmissionToken {
    /// Creates an admission token from a non-empty stable string.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            return Err(StoreError::Identity(
                "admission token must not be empty".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Returns the stable token string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Operational admission lease for nowait lanes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionLease {
    /// Lane held by the lease.
    pub lane: AdmissionLaneKey,
    /// Holder token.
    pub token: AdmissionToken,
    /// Lease expiry as milliseconds since the Unix epoch.
    pub lease_expires_at_unix_ms: i64,
}

/// Deterministic operational waiter id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdmissionWaiterId(String);

impl AdmissionWaiterId {
    /// Creates a waiter id from a non-empty stable string.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            return Err(StoreError::Identity(
                "admission waiter id must not be empty".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Returns the stable waiter id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Non-authoritative waiter metadata for FIFO admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionWaiter {
    /// Deterministic waiter id for the stable admission token.
    pub waiter_id: AdmissionWaiterId,
    /// Lane-local FIFO ticket assigned under the lane admission lock.
    pub lane_ticket: u64,
    /// Lease expiry as milliseconds since the Unix epoch.
    pub lease_expires_at_unix_ms: i64,
}

/// Successful FIFO admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitFifoAdmissionGrant {
    /// Admitted FIFO lane.
    pub lane: ResourceAdmissionLane,
}

/// FIFO admission result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitFifoAdmissionResult {
    /// The claim was admitted.
    Admitted(WaitFifoAdmissionGrant),
    /// The claim was blocked before domain authority was persisted.
    Blocked(Box<WaitFifoAdmissionBlock>),
}

/// Result of a FIFO claim blocked before domain authority rows were persisted.
///
/// A blocked claim persists no run event, commit, resource-lane claim, resource-lane release,
/// lane-transition, or other MFM domain authority row. Concrete stores may insert or refresh
/// non-authoritative operational waiter rows for FIFO admission; those rows are coordination
/// state only and never grant lane ownership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitFifoAdmissionBlock {
    /// Blocked resource-admission lane.
    pub lane: ResourceAdmissionLane,
    /// Normalized resource lane key for status and diagnostics.
    pub resource_lane_key: ResourceLaneKey,
    /// Current authoritative holder of the lane, if one exists.
    pub holder: Option<SideEffectLedgerRef>,
    /// Operational waiter metadata for the blocked claim, if the store exposed it.
    pub waiter: Option<AdmissionWaiter>,
}

/// Nowait admission result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NowaitSkipAdmissionResult {
    /// The claim was admitted.
    Admitted(AdmissionLease),
    /// The lane is currently busy.
    Busy(NowaitSkipAdmissionBusy),
}

/// Busy result for a nowait lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowaitSkipAdmissionBusy {
    /// Busy execution-claim lane.
    pub lane: ExecutionClaimAdmissionLane,
    /// Current holder lease if the store can expose it.
    pub holder: Option<AdmissionLease>,
}

/// Read-only execution-claim status for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionClaimStatus {
    /// No holder token is present for this run's execution-claim lane.
    Unclaimed,
    /// A holder token is present and its lease has not expired.
    Live(AdmissionLease),
    /// A holder token is present but its lease has expired and needs explicit reaping.
    Expired(AdmissionLease),
}

/// Expired execution claim exposed for explicit manual reaping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredExecutionClaim {
    /// Run whose execution claim has expired.
    pub run_id: RunId,
    /// Expired execution-claim lane.
    pub lane: ExecutionClaimAdmissionLane,
    /// Expired holder lease.
    pub lease: AdmissionLease,
}

/// PostgreSQL `pg_advisory_xact_lock(int8)` key for an admission lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdmissionAdvisoryLockKey(i64);

impl AdmissionAdvisoryLockKey {
    /// Returns the signed 64-bit advisory lock key.
    pub fn as_i64(self) -> i64 {
        self.0
    }
}

/// Returns canonical JSON bytes for a resource key evidence value.
pub fn resource_key_canonical_json(evidence: &events::ResourceKeyEvidence) -> Result<Vec<u8>> {
    Ok(canonical_json(serde_json::json!({
        "key": evidence.key.as_str(),
    }))?
    .to_vec())
}

/// Derives the stable FIFO admission token for a resource-lane claim intent.
pub fn resource_wait_fifo_admission_token(
    run_id: &RunId,
    lane: &ResourceAdmissionLane,
    intent: &events::ResourceLaneClaimIntent,
) -> Result<AdmissionToken> {
    let canonical = canonical_json(serde_json::json!({
        "attempt_id": intent.attempt_id.as_str(),
        "domain": String::from_utf8_lossy(RESOURCE_WAIT_FIFO_TOKEN_DOMAIN),
        "invocation_epoch": intent.invocation_epoch,
        "lane_id": bytes_hex(lane.id().as_bytes()),
        "ledger_key": intent.ledger_key.as_str(),
        "node_id": intent.node_id.as_str(),
        "pair_id": intent.pair_id.as_ref().map(mfm_ids::SideEffectPairId::as_str),
        "pair_role": intent.pair_role.map(events::SideEffectPairRole::as_str),
        "requirement_digest": intent.requirement_digest.as_str(),
        "resolved_by_capability_impl": intent.resolved_by_capability_impl.as_str(),
        "run_id": run_id.as_str(),
    }))?;
    let mut input =
        Vec::with_capacity(RESOURCE_WAIT_FIFO_TOKEN_DOMAIN.len() + canonical.as_bytes().len());
    input.extend_from_slice(RESOURCE_WAIT_FIFO_TOKEN_DOMAIN);
    input.extend_from_slice(canonical.as_bytes());
    AdmissionToken::new(bytes_hex(sha256_digest_bytes(&input).as_bytes()))
}

/// Derives the stable FIFO waiter id from an admission token.
pub fn admission_waiter_id(token: &AdmissionToken) -> Result<AdmissionWaiterId> {
    let mut input = Vec::with_capacity(ADMISSION_WAITER_ID_DOMAIN.len() + token.as_str().len());
    input.extend_from_slice(ADMISSION_WAITER_ID_DOMAIN);
    input.extend_from_slice(token.as_str().as_bytes());
    AdmissionWaiterId::new(format!(
        "admission_waiter:{}",
        bytes_hex(sha256_digest_bytes(&input).as_bytes())
    ))
}

/// Derives the signed 64-bit advisory lock key for an admission lane.
pub fn admission_advisory_lock_key(lane: &AdmissionLaneKey) -> Result<AdmissionAdvisoryLockKey> {
    let canonical = canonical_json(serde_json::json!({
        "class": lane.class().as_str(),
        "domain": String::from_utf8_lossy(ADMISSION_ADVISORY_LOCK_DOMAIN),
        "lane_id": bytes_hex(lane.id().as_bytes()),
    }))?;
    let mut input =
        Vec::with_capacity(ADMISSION_ADVISORY_LOCK_DOMAIN.len() + canonical.as_bytes().len());
    input.extend_from_slice(ADMISSION_ADVISORY_LOCK_DOMAIN);
    input.extend_from_slice(canonical.as_bytes());
    let digest = sha256_digest_bytes(&input);
    let mut key = [0_u8; 8];
    key.copy_from_slice(&digest.as_bytes()[..8]);
    Ok(AdmissionAdvisoryLockKey(i64::from_be_bytes(key)))
}

fn bytes_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
