use super::*;

/// Stable semantic run mode for typed run responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunModeStatus {
    /// Forward graph execution is active.
    Forward,
    /// Remediation graph execution is active.
    Remediating,
    /// The run is blocked for certified manual evidence.
    ManualBlocked,
    /// The run completed with public-output evidence.
    Completed,
    /// Confirmed forward obligations were compensated.
    Compensated,
    /// Certified manual evidence resolved the run.
    ManuallyResolved,
    /// The run ended without a compensation or AC/DC-equivalence claim.
    FailedWithoutAcdcClaim,
}

impl RunModeStatus {
    fn as_store_run_mode(self) -> store::RunMode {
        match self {
            Self::Forward => store::RunMode::Forward,
            Self::Remediating => store::RunMode::Remediating,
            Self::ManualBlocked => store::RunMode::ManualBlocked,
            Self::Completed => store::RunMode::Completed,
            Self::Compensated => store::RunMode::Compensated,
            Self::ManuallyResolved => store::RunMode::ManuallyResolved,
            Self::FailedWithoutAcdcClaim => store::RunMode::FailedWithoutAcdcClaim,
        }
    }
}

impl From<store::RunMode> for RunModeStatus {
    fn from(mode: store::RunMode) -> Self {
        match mode {
            store::RunMode::Forward => Self::Forward,
            store::RunMode::Remediating => Self::Remediating,
            store::RunMode::ManualBlocked => Self::ManualBlocked,
            store::RunMode::Completed => Self::Completed,
            store::RunMode::Compensated => Self::Compensated,
            store::RunMode::ManuallyResolved => Self::ManuallyResolved,
            store::RunMode::FailedWithoutAcdcClaim => Self::FailedWithoutAcdcClaim,
        }
    }
}

impl fmt::Display for RunModeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_store_run_mode().as_str())
    }
}

/// Public saga status derived from certified policy and stream evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SagaStatus {
    /// Certified saga policy variant and manual authorization requirements.
    pub policy: SagaPolicyStatus,
    /// Derived per-forward-ledger obligations.
    pub obligations: Vec<SagaObligationStatus>,
    /// Resource claims and recorded evidence for all projected side-effect ledgers.
    pub resource_ledgers: Vec<ResourceLedgerStatus>,
    /// Active exclusive resource lane holders referenced by this run's live side-effect ledgers.
    pub resource_lanes: Vec<ResourceLaneHolderStatus>,
    /// Manual-block reason when the derived run mode is `manual_blocked`.
    pub manual_block_reason: Option<String>,
    /// Required manual authorization when an authorized decision can resolve the current block.
    pub required_manual_authorization: Option<ManualAuthorizationRequirements>,
    /// Terminal completion evidence, when the run has resolved.
    pub terminal_resolution: Option<TerminalResolutionStatus>,
}

/// Public certified saga policy summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SagaPolicyStatus {
    /// Certified policy variant.
    pub variant: String,
    /// Manual authorization requirements certified directly by the policy, when present.
    pub manual_authorization: Option<ManualAuthorizationRequirements>,
    /// Unresolved-remediation directive under compensating policy.
    pub on_remediation_unresolved: Option<String>,
}

/// Public manual authorization requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualAuthorizationRequirements {
    /// Schema id for the operator evidence artifact.
    pub evidence_schema_id: String,
    /// Manual authorization verifier id.
    pub verifier_id: String,
    /// Required manual signing scheme.
    pub signing_scheme: String,
    /// Certified operator authority id.
    pub authority_id: String,
    /// Allowed operator public identities from the certified authority snapshot.
    pub operator_public_identities: Vec<String>,
    /// Required number of operator signatures.
    pub quorum_required_signatures: u32,
}

/// Public obligation state for one forward ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SagaObligationStatus {
    /// Forward ledger key.
    pub forward_ledger_key: String,
    /// Current forward side-effect phase.
    pub forward_phase: String,
    /// Derived forward classification.
    pub classification: String,
    /// Declared resource claim and recorded evidence for the forward ledger.
    pub resource: Option<ResourceLedgerStatus>,
    /// Linked remediation ledger, when one exists.
    pub remediation: Option<RemediationLedgerStatus>,
}

/// Public remediation state linked to a forward ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemediationLedgerStatus {
    /// Remediation ledger key.
    pub ledger_key: String,
    /// Forward ledger key this remediation closes.
    pub forward_ledger_key: String,
    /// Current remediation side-effect phase.
    pub phase: String,
    /// Declared resource claim and recorded evidence for the remediation ledger.
    pub resource: Option<ResourceLedgerStatus>,
    /// Whether remediation confirmation closed the obligation.
    pub closed: bool,
    /// Unresolved reason if remediation cannot close the obligation.
    pub unresolved: Option<String>,
}

/// Public resource-claim and evidence status for one side-effect ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLedgerStatus {
    /// Side-effect ledger key.
    pub ledger_key: String,
    /// Ledger purpose.
    pub ledger_purpose: String,
    /// Linked forward ledger key when this is a remediation ledger.
    pub forward_ledger_key: Option<String>,
    /// Current projected side-effect phase.
    pub phase: String,
    /// Certified resource claim declared by the side-effect node.
    pub claim: ResourceClaimStatus,
    /// Recorded exclusive key evidence, when the claim is `exclusive` and preparation occurred.
    pub key: Option<ResourceKeyStatus>,
    /// Recorded exact touched-set evidence, when the claim is `exact_touched_set`.
    pub touched_set: Option<ResourceTouchedSetStatus>,
    /// Active lane holder when this ledger currently owns its exclusive lane.
    pub active_lane: Option<ResourceLaneHolderStatus>,
    /// Active holder of the same recorded key when this ledger is not the holder.
    pub blocked_by_lane: Option<ResourceLaneHolderStatus>,
}

/// Public certified resource claim summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceClaimStatus {
    /// Claim variant: `exclusive`, `exact_touched_set`, or `manual_only`.
    pub kind: String,
    /// Resource namespace for `exclusive` and `exact_touched_set` claims.
    pub namespace: Option<String>,
    /// Key schema id for `exclusive` claims.
    pub key_schema_id: Option<String>,
    /// Evidence schema id for `exact_touched_set` claims.
    pub evidence_schema_id: Option<String>,
}

/// Public exclusive resource key evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceKeyStatus {
    /// Resource namespace.
    pub namespace: String,
    /// Key schema id.
    pub key_schema_id: String,
    /// Stable digest of the store-comparable resource key evidence.
    pub key_digest: String,
}

/// Public exact touched-set evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceTouchedSetStatus {
    /// Resource namespace.
    pub namespace: String,
    /// Evidence schema id.
    pub evidence_schema_id: String,
    /// Canonical touched-set evidence hash.
    pub evidence_hash: String,
    /// Touched-set evidence artifact id.
    pub evidence_artifact_id: String,
}

/// Public active exclusive resource lane holder referenced by the target run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLaneHolderStatus {
    /// Resource namespace.
    pub namespace: String,
    /// Key schema id.
    pub key_schema_id: String,
    /// Stable digest of the store-comparable resource lane key.
    pub key_digest: String,
    /// Run id holding the lane.
    pub holding_run_id: String,
    /// Ledger key holding the lane.
    pub holding_ledger_key: String,
    /// Ledger purpose for the holder.
    pub holding_ledger_purpose: String,
    /// Forward ledger key when the holder is a remediation ledger.
    pub holding_forward_ledger_key: Option<String>,
    /// Node id that prepared the invocation.
    pub holding_node_id: String,
    /// Attempt id that prepared the invocation.
    pub holding_attempt_id: String,
    /// Invocation epoch that prepared the invocation.
    pub invocation_epoch: u32,
}

/// Public attempt lifecycle disposition derived from committed attempt projections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptDispositionStatus {
    /// Node id that owns the attempt.
    pub node_id: String,
    /// Attempt id.
    pub attempt_id: String,
    /// Attempt disposition: `started`, `completed`, `failed`, or `interrupted`.
    pub disposition: String,
    /// Attempt number when the attempt is still open.
    pub attempt_no: Option<u32>,
    /// Retryability for failed attempts.
    pub retryable: Option<bool>,
    /// Stable error code for failed attempts.
    pub error_code: Option<String>,
    /// Output cell id for completed attempts.
    pub output_cell_id: Option<String>,
}

/// Public terminal resolution summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalResolutionStatus {
    /// Terminal outcome.
    pub outcome: String,
    /// Public claim carried by the terminal outcome.
    pub claim: String,
}

/// Response returned after typed start or resume dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunResponse {
    /// Run id.
    pub run_id: String,
    /// Certified spec hash.
    pub spec_hash: String,
    /// Current semantic run mode.
    pub run_mode: RunModeStatus,
    /// Derived saga status.
    pub saga: SagaStatus,
    /// Attempt-level dispositions, distinct from semantic run mode.
    pub attempt_dispositions: Vec<AttemptDispositionStatus>,
    /// Last scheduler status observed by the app dispatch loop.
    ///
    /// Read-only status reports `observed`. Start/resume dispatch reports scheduler progress or
    /// execution-claim coordination, and already-terminal resume reports `observed` because no
    /// scheduler dispatch is needed.
    pub scheduler_status: String,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
}

impl fmt::Display for RunResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "run {} run_mode={} spec_hash={} head_seq={} scheduler_status={}",
            self.run_id, self.run_mode, self.spec_hash, self.head_seq, self.scheduler_status
        )
    }
}

/// Public launch outcome kind for start responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLaunchOutcomeStatus {
    /// A new run was admitted by this launch request.
    Admitted,
    /// The launch request attached to an already admitted compatible run.
    Attached,
    /// The launch request found active equivalent work and did not admit a new run.
    AlreadyActive,
}

impl RunLaunchOutcomeStatus {
    /// Returns the stable public status string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Attached => "attached",
            Self::AlreadyActive => "already_active",
        }
    }
}

impl fmt::Display for RunLaunchOutcomeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// App-level result of a normal run launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunLaunchOutcome {
    /// This launch admitted a new run.
    Admitted {
        /// Current run status after driving until blocked or completed.
        run: RunResponse,
    },
    /// This launch attached to an already admitted compatible run.
    Attached {
        /// Current run status for the existing run.
        run: RunResponse,
    },
    /// This launch found active equivalent work before run admission.
    AlreadyActive {
        /// Active run id holding the equivalent execution lane.
        active_run_id: RunId,
    },
}

impl RunLaunchOutcome {
    /// Returns the stable public outcome kind.
    pub fn status(&self) -> RunLaunchOutcomeStatus {
        match self {
            Self::Admitted { .. } => RunLaunchOutcomeStatus::Admitted,
            Self::Attached { .. } => RunLaunchOutcomeStatus::Attached,
            Self::AlreadyActive { .. } => RunLaunchOutcomeStatus::AlreadyActive,
        }
    }

    /// Returns the run response carried by this outcome.
    pub fn run(&self) -> Option<&RunResponse> {
        match self {
            Self::Admitted { run } | Self::Attached { run } => Some(run),
            Self::AlreadyActive { .. } => None,
        }
    }

    /// Splits this outcome into the public kind, run response, and active run id.
    pub fn into_response_parts(
        self,
    ) -> (RunLaunchOutcomeStatus, Option<RunResponse>, Option<RunId>) {
        match self {
            Self::Admitted { run } => (RunLaunchOutcomeStatus::Admitted, Some(run), None),
            Self::Attached { run } => (RunLaunchOutcomeStatus::Attached, Some(run), None),
            Self::AlreadyActive { active_run_id } => (
                RunLaunchOutcomeStatus::AlreadyActive,
                None,
                Some(active_run_id),
            ),
        }
    }
}

/// Typed public-output rendering response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicOutputResponse {
    /// Run id.
    pub run_id: String,
    /// Public output schema id.
    pub public_schema_id: String,
    /// Producing public-output event id.
    pub event_id: String,
    /// Canonical rendered output digest.
    pub rendered_digest: String,
    /// Rendered artifact id when the renderer persisted output bytes.
    pub rendered_artifact_id: Option<String>,
    /// Rendered JSON body loaded from the typed artifact store, when available and JSON encoded.
    pub json: Option<serde_json::Value>,
}

impl fmt::Display for PublicOutputResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.json {
            Some(json) => write!(
                f,
                "{}",
                serde_json::to_string(json).map_err(|_| fmt::Error)?
            ),
            None => write!(
                f,
                "run {} public_schema_id={} event_id={} rendered_digest={}",
                self.run_id, self.public_schema_id, self.event_id, self.rendered_digest
            ),
        }
    }
}

/// App-level report for a start request after launch driving and optional public-output rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStartReport {
    /// Public launch outcome kind.
    pub outcome: RunLaunchOutcomeStatus,
    /// Current run status when a run was admitted or attached.
    pub run: Option<RunResponse>,
    /// Active equivalent run id when admission was skipped.
    pub active_run_id: Option<String>,
    /// Rendered public output when the run completed during launch.
    pub public_output: Option<PublicOutputResponse>,
}

/// Non-forgeable authority to render one typed public output.
///
/// This is minted only while borrowing the store-owned verified run view after the app validates
/// its current public-output evidence. Rendered JSON artifacts are caches only and cannot construct
/// this authority.
#[derive(Debug)]
pub struct PublicOutputReadAuthority<'view> {
    pub(super) run_id: &'view RunId,
    pub(super) public_schema_id: &'view SchemaId,
    pub(super) event_id: &'view EventId,
    pub(super) rendered_digest: &'view ContentDigest,
    pub(super) rendered_artifact_id: Option<&'view ArtifactId>,
    pub(super) payload: &'view events::PublicOutputProduced,
}

impl PublicOutputReadAuthority<'_> {
    /// Returns the run id bound to this read authority.
    pub fn run_id(&self) -> &RunId {
        self.run_id
    }

    /// Returns the public-output schema id bound to this read authority.
    pub fn public_schema_id(&self) -> &SchemaId {
        self.public_schema_id
    }

    /// Returns the store-owned event id that produced this public output.
    pub fn event_id(&self) -> &EventId {
        self.event_id
    }

    /// Returns the canonical digest of the rendered public output.
    pub fn rendered_digest(&self) -> &ContentDigest {
        self.rendered_digest
    }

    /// Returns the persisted rendered artifact id, when the renderer wrote one.
    pub fn rendered_artifact_id(&self) -> Option<&ArtifactId> {
        self.rendered_artifact_id
    }
}

/// Typed run event stream response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunStreamResponse {
    /// Run id.
    pub run_id: String,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
    /// Store-owned typed event references.
    pub events: Vec<RunEventRef>,
}

impl fmt::Display for RunStreamResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "run {} head_seq={} events={}",
            self.run_id,
            self.head_seq,
            self.events.len()
        )
    }
}

/// Transport-safe reference to one store-owned typed event envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunEventRef {
    /// Store-derived event id.
    pub event_id: String,
    /// Event schema id.
    pub event_schema_id: String,
    /// Store-owned stream sequence.
    pub seq: u64,
    /// Store-owned ordinal within the atomic commit.
    pub ordinal: u32,
    /// Commit key that appended this event.
    pub commit_key: String,
    /// Store-derived logical key.
    pub logical_key: String,
    /// Canonical payload hash.
    pub payload_hash: String,
    /// Stable error code when this event is a failed state attempt.
    pub error_code: Option<String>,
}

/// Response returned after verifying replay authority for a typed run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayResponse {
    /// Run id.
    pub run_id: String,
    /// Certified spec hash.
    pub spec_hash: String,
    /// Current semantic run mode.
    pub run_mode: RunModeStatus,
    /// Derived saga status.
    pub saga: SagaStatus,
    /// Attempt-level dispositions, distinct from semantic run mode.
    pub attempt_dispositions: Vec<AttemptDispositionStatus>,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
    /// Retained artifact evidence entries supplied to the replay broker.
    pub retained_artifacts: usize,
}

impl fmt::Display for ReplayResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "run {} replay verified spec_hash={} run_mode={} head_seq={} retained_artifacts={}",
            self.run_id, self.spec_hash, self.run_mode, self.head_seq, self.retained_artifacts
        )
    }
}

pub(super) fn verify_replay_diagnostics_from_recorded_artifacts(
    view: &store::VerifiedRunView,
) -> Result<(), PublicError> {
    let lifecycle = store::current_lifecycle::read(view);
    let mut failures = Vec::new();
    let _ = lifecycle.visit_records(|record| {
        if let store::current_lifecycle::CurrentRecordKindRef::StateAttemptFailed(payload) =
            record.kind()
        {
            let mut requirement = None;
            let _ = record.visit_artifact_requirements(|candidate| {
                if candidate.source
                    == store::EventArtifactReferenceSource::StateAttemptFailureDiagnostic
                {
                    requirement = Some(candidate.clone());
                    std::ops::ControlFlow::Break(())
                } else {
                    std::ops::ControlFlow::Continue(())
                }
            });
            failures.push((payload, requirement));
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    for (payload, requirement) in failures {
        let Some(diagnostic_ref) = &payload.error.diagnostic_ref else {
            continue;
        };
        let requirement = requirement.ok_or_else(replay_diagnostic_error)?;
        if requirement.artifact_id != diagnostic_ref.artifact_id
            || requirement.evidence_hash != diagnostic_ref.evidence_hash
        {
            return Err(replay_diagnostic_error());
        }
        let artifact = lifecycle
            .object_for_requirement(&requirement)
            .ok_or_else(replay_diagnostic_error)?;
        let diagnostic_artifact = serde_json::from_slice::<serde_json::Value>(artifact.bytes())
            .map_err(|_| replay_diagnostic_error())?;
        let diagnostics =
            mfm_runtime::attempt_failure_diagnostics_from_artifact_json(&diagnostic_artifact)
                .map_err(|_| replay_diagnostic_error())?;
        verify_replay_diagnostic(payload.error.public_details.as_ref(), &diagnostics)?;
    }
    Ok(())
}
