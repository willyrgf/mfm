#![warn(missing_docs)]
//! Fixed-tenant application facade for MFM.
//!
//! Trusted embedding selects the tenant once when constructing `Application`.  Public methods
//! accept no credential, principal, policy, or tenant override.  Cross-tenant lookup is the same
//! redacted `RunNotFound` result as an absent run.

use std::collections::BTreeMap;
use std::sync::Mutex;

use mfm_canonical::{raw_content_digest, sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::AccessCapabilityContract;
use mfm_evm::{
    BroadcastEvidence, BroadcastIntent, BroadcastTransaction, EvmBalanceAsset,
    EvmBalanceCollectionCompletion, EvmBalanceContext, EvmConfig, EvmNativeBalanceInput,
    EvmReadEvidence, EvmReadIntent, EvmSubmissionContext, EvmSubmissionFailure,
    EvmSubmissionOutput, EvmSubmissionRequest, EvmTokenBalanceInput, ReadBalance,
    ReadWalletNonceStatus, EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID,
};
#[cfg(test)]
use mfm_ids::{AppendRequestId, StoreEpoch, StoreScopeId};
use mfm_ids::{
    ContentRef, DigestAlgorithm, RunId, SchemaId, SequentialControlAddress, StableId, TenantScopeId,
};
use mfm_journal::single_trust::{RunFrame, RunRecord};
use mfm_portfolio::{
    PortfolioConfig, PortfolioContinuation, PortfolioSnapshotFailure, PortfolioSnapshotInput,
    PortfolioSnapshotOutput, PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
};
use mfm_program::single_trust::{
    nominal_contract_ref, Declaration, ExecutionMode, MatchDeclaration, MatchVariant,
    ProgramCatalog, ProgramCatalogBuilder, ProgramDocument, StateDeclaration,
};
use mfm_replay::{qualify_with_program, PortableRun, ReplayError, ReplayReport};
use mfm_runtime::{ResumeStep, Runtime, RuntimeStep, SpawnStep, SuspendedRun};
use mfm_store::{
    ConfigurationStore, HistoryReader, PreparedAdmission, QualifiedHistoryPort,
    ResolvedConfigurationHead, StoreAuditPort,
};
use mfm_store::{RunAction, StoreError};
#[cfg(test)]
use mfm_store::{StoreWorkLimits, StructuredStore, StructuredStoreIdentity};
use mfm_values::{string_contains_secret_marker, MfmConfig, MfmValue};
use serde::{Deserialize, Serialize};

/// Maximum canonical admission body accepted by every transport.
pub const MAX_ADMISSION_BYTES: usize = 512 * 1024;

/// Result type for fixed-tenant application operations.
pub type Result<T> = std::result::Result<T, PublicError>;

/// Redaction-safe application error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PublicError {
    /// The caller supplied malformed or unsupported input.
    #[error("{code}: {message}")]
    BadRequest {
        /// Stable public error code.
        code: &'static str,
        /// Redaction-safe public detail.
        message: &'static str,
    },
    /// The run is absent from this fixed tenant partition.
    #[error("RunNotFound: run was not found")]
    RunNotFound,
    /// The Store or retained prefix is invalid.
    #[error("Internal: application state is unavailable")]
    Internal,
}

impl PublicError {
    /// Returns the stable public error code.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::BadRequest { code, .. } => code,
            Self::RunNotFound => "RunNotFound",
            Self::Internal => "Internal",
        }
    }
}

/// Strict singular admission request.
#[derive(Debug, Clone)]
pub struct AdmitRunRequest {
    /// Exact supported entry-point identity.
    entry_point_id: StableId,
    /// One domain-planned singular `C0` JSON value.
    input: serde_json::Value,
}

impl AdmitRunRequest {
    /// Validates a request from an already parsed transport value.
    pub fn new(entry_point_id: StableId, input: serde_json::Value) -> Result<Self> {
        if contains_float(&input) || contains_secret_marker(&input) {
            return Err(PublicError::BadRequest {
                code: "AdmissionRequestInvalid",
                message: "Admission input is not a bounded secret-free canonical value",
            });
        }
        let encoded = serde_json::to_string(&input).map_err(|_| PublicError::Internal)?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&encoded).map_err(|_| {
            PublicError::BadRequest {
                code: "AdmissionRequestInvalid",
                message: "Admission input is not canonical JSON",
            }
        })?;
        if canonical.as_bytes().len() > MAX_ADMISSION_BYTES {
            return Err(PublicError::BadRequest {
                code: "AdmissionRequestInvalid",
                message: "Admission input exceeds its bounded size",
            });
        }
        Ok(Self {
            entry_point_id,
            input,
        })
    }
}

/// Parses one admission JSON value after rejecting duplicate keys, floats, and oversized input.
pub fn parse_admission_json(input: &str) -> Result<serde_json::Value> {
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(input).map_err(|_| PublicError::BadRequest {
            code: "AdmissionRequestInvalid",
            message: "Admission input is not strict canonical JSON",
        })?;
    if canonical.as_bytes().len() > MAX_ADMISSION_BYTES {
        return Err(PublicError::BadRequest {
            code: "AdmissionRequestInvalid",
            message: "Admission input exceeds its bounded size",
        });
    }
    serde_json::from_str(input).map_err(|_| PublicError::BadRequest {
        code: "AdmissionRequestInvalid",
        message: "Admission input is not a JSON value",
    })
}

/// Public admission response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmitRunResponse {
    /// Durable run identity.
    pub run_id: RunId,
    /// Mechanical admission disposition.
    pub disposition: &'static str,
}

/// Public one-step drive response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveResponse {
    /// Durable run identity.
    pub run_id: RunId,
    /// Current retained head.
    pub head_sequence: u64,
    /// Callback-free status.
    pub status: RunStatus,
}

/// Fixed-tenant public run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    /// The run has a callback-free actionable State.
    Ready,
    /// One Access preparation is durable and selected.
    WaitingPreparation,
    /// A durable terminal conclusion exists.
    Terminal,
    /// A durable typed failure stopped the sequential path.
    Failed,
}

/// Ordinary public run view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicRunView {
    /// Durable run identity.
    pub run_id: RunId,
    /// Fixed tenant partition selected at construction.
    pub tenant_scope_id: TenantScopeId,
    /// Current head sequence.
    pub head_sequence: u64,
    /// Callback-free status.
    pub status: RunStatus,
}

/// Callback-free replay response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayResponse {
    /// Qualified head sequence.
    pub head_sequence: u64,
    /// Derived terminal marker.
    pub terminal: bool,
}

/// Callback-free trace of the retained three-family stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceResponse {
    /// Durable run identity.
    pub run_id: RunId,
    /// Qualified retained head.
    pub head_sequence: u64,
    /// One redacted structural entry per retained frame.
    pub records: Vec<TraceRecord>,
}

/// Redacted structural trace entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceRecord {
    /// Sole genesis frame.
    Admitted,
    /// Durable Access preparation.
    Prepared {
        /// Exact sequential occurrence.
        occurrence: SequentialControlAddress,
        /// Preparation attempt ordinal.
        preparation_ordinal: u16,
    },
    /// Durable State conclusion.
    Concluded {
        /// Exact sequential occurrence.
        occurrence: SequentialControlAddress,
        /// Whether the conclusion carries a successful successor/root result.
        success: bool,
    },
}

/// Callback-free structural Access audit derived from retained frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessAuditResponse {
    /// Durable run identity.
    pub run_id: RunId,
    /// Qualified retained head.
    pub head_sequence: u64,
    /// One entry for each Access occurrence present in retained history.
    pub entries: Vec<AccessAuditEntry>,
}

/// Redacted preparation/conclusion status for one Access occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessAuditEntry {
    /// Exact sequential occurrence.
    pub occurrence: SequentialControlAddress,
    /// Number of retained preparations.
    pub preparation_count: u16,
    /// Latest retained preparation ordinal, when present.
    pub latest_preparation_ordinal: Option<u16>,
    /// Whether the occurrence has a durable conclusion.
    pub concluded: bool,
}

impl From<ReplayReport> for ReplayResponse {
    fn from(value: ReplayReport) -> Self {
        Self {
            head_sequence: value.head_sequence(),
            terminal: value.terminal(),
        }
    }
}

/// Strict portable v5 run export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedRun {
    bytes: Vec<u8>,
}

impl ExportedRun {
    /// Returns the canonical v5 structural envelope bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Fixed-tenant application facade.
pub struct Application {
    tenant_scope_id: TenantScopeId,
    history: QualifiedHistoryPort,
    reader: HistoryReader,
    _configuration: ConfigurationStore,
    _audit: StoreAuditPort,
    catalog: ProgramCatalog,
    supported_entry_points: BTreeMap<StableId, ()>,
    configuration_heads: BTreeMap<StableId, ResolvedConfigurationHead>,
    runtimes: BTreeMap<StableId, Runtime>,
    suspended: Mutex<BTreeMap<RunId, ApplicationSuspended>>,
}

enum ApplicationSuspended {
    Runtime(Box<SuspendedRun>),
    Admission(Box<PreparedAdmission>),
}

impl Application {
    /// Composes one facade from explicit Store ports and per-entry configuration heads.
    pub fn new(
        history: QualifiedHistoryPort,
        reader: HistoryReader,
        configuration: ConfigurationStore,
        audit: StoreAuditPort,
        portfolio_configuration: ResolvedConfigurationHead,
        evm_configuration: ResolvedConfigurationHead,
        runtimes: Vec<Runtime>,
    ) -> Result<Self> {
        let portfolio_id =
            StableId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).map_err(|_| PublicError::Internal)?;
        let evm_id = StableId::new(EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID)
            .map_err(|_| PublicError::Internal)?;
        if portfolio_configuration.content_ref().schema_id()
            != &<PortfolioConfig as MfmConfig>::schema_id().map_err(|_| PublicError::Internal)?
            || evm_configuration.content_ref().schema_id()
                != &EvmConfig::schema_id().map_err(|_| PublicError::Internal)?
        {
            return Err(PublicError::Internal);
        }
        let mut runtime_map = BTreeMap::new();
        for runtime in runtimes {
            let configuration_head = if runtime.entry_point_id() == portfolio_id {
                &portfolio_configuration
            } else {
                &evm_configuration
            };
            if history.identity() != runtime.store_identity()
                || !history.catalog().same_catalog(&runtime.catalog())
                || !runtime.accepts_configuration_head(configuration_head)
            {
                return Err(PublicError::Internal);
            }
            let entry_point = runtime.entry_point_id();
            if runtime_map.insert(entry_point, runtime).is_some() {
                return Err(PublicError::Internal);
            }
        }
        if runtime_map
            .keys()
            .any(|entry| entry != &portfolio_id && entry != &evm_id)
        {
            return Err(PublicError::Internal);
        }
        if (!runtime_map.contains_key(&portfolio_id)
            && !configuration.owns_head(&portfolio_configuration))
            || (!runtime_map.contains_key(&evm_id) && !configuration.owns_head(&evm_configuration))
        {
            return Err(PublicError::Internal);
        }
        let identity = history.identity().clone();
        let tenant_scope_id = identity.tenant().clone();
        let catalog = history.catalog().clone();
        Ok(Self {
            history,
            reader,
            _configuration: configuration,
            _audit: audit,
            catalog,
            tenant_scope_id,
            supported_entry_points: BTreeMap::from([
                (portfolio_id.clone(), ()),
                (evm_id.clone(), ()),
            ]),
            configuration_heads: BTreeMap::from([
                (portfolio_id.clone(), portfolio_configuration),
                (evm_id.clone(), evm_configuration),
            ]),
            runtimes: runtime_map,
            suspended: Mutex::new(BTreeMap::new()),
        })
    }

    /// Returns the fixed tenant selected by trusted embedding.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the two supported entry-point identities.
    pub fn entry_points(&self) -> Result<[StableId; 2]> {
        Ok([
            StableId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).map_err(|_| PublicError::Internal)?,
            StableId::new(EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID)
                .map_err(|_| PublicError::Internal)?,
        ])
    }

    /// Checks the admitted Store identity without opening retained history.
    pub async fn check_ready(&self) -> Result<()> {
        (self.supported_entry_points.len() == 2)
            .then_some(())
            .ok_or(PublicError::Internal)
    }

    /// Admits exactly one singular `C0` and one genesis frame.
    pub async fn admit_run(&self, request: AdmitRunRequest) -> Result<AdmitRunResponse> {
        let entry_point_id = request.entry_point_id.clone();
        if !self.supported_entry_points.contains_key(&entry_point_id) {
            return Err(PublicError::BadRequest {
                code: "EntryPointNotFound",
                message: "The entry point is not registered",
            });
        }
        let identity_input = request.input.clone();
        let (canonical, contract_ref, document) =
            canonical_typed_admission(&entry_point_id, request.input)?;
        let program = self
            .catalog
            .program(document.clone())
            .map_err(|_| PublicError::Internal)?;
        let identity = if entry_point_id.as_str() == EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID {
            let typed: EvmSubmissionRequest = serde_json::from_value(identity_input.clone())
                .map_err(|_| PublicError::Internal)?;
            canonical_json(&serde_json::json!({
                "entry_point_id": entry_point_id.as_str(),
                "idempotency_key": typed.idempotency_key,
                "nonce_domain": typed.target.nonce_domain,
                "tenant_scope_id": self.tenant_scope_id.as_str(),
            }))?
        } else {
            canonical_json(&serde_json::json!({
                "entry_point_id": entry_point_id.as_str(),
                "input_digest": raw_content_digest(canonical.as_bytes()).as_str(),
                "tenant_scope_id": self.tenant_scope_id.as_str(),
            }))?
        };
        let run_id = RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(identity.as_bytes()),
        );
        let configuration = self
            .configuration_heads
            .get(&entry_point_id)
            .ok_or(PublicError::Internal)?
            .clone();
        if let Some(runtime) = self.runtimes.get(&entry_point_id) {
            let step = if entry_point_id.as_str() == EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID {
                let typed: EvmSubmissionRequest =
                    serde_json::from_value(identity_input).map_err(|_| PublicError::Internal)?;
                let value = runtime
                    .catalog()
                    .qualify(contract_ref.clone(), typed)
                    .map_err(|_| PublicError::Internal)?;
                runtime
                    .admission(run_id.clone(), value, configuration.clone(), Vec::new())
                    .map_err(|_| PublicError::Internal)?
                    .spawn()
                    .await
            } else {
                let typed: PortfolioSnapshotInput =
                    serde_json::from_value(identity_input).map_err(|_| PublicError::Internal)?;
                let value = runtime
                    .catalog()
                    .qualify(contract_ref.clone(), typed)
                    .map_err(|_| PublicError::Internal)?;
                runtime
                    .admission(run_id.clone(), value, configuration.clone(), Vec::new())
                    .map_err(|_| PublicError::Internal)?
                    .spawn()
                    .await
            };
            let disposition = match step {
                SpawnStep::Active(_) => "accepted",
                SpawnStep::Terminal(_) => "terminal",
                SpawnStep::Suspended(suspended) => {
                    let mut owners = self.suspended.lock().map_err(|_| PublicError::Internal)?;
                    if owners.contains_key(&run_id) {
                        return Err(PublicError::Internal);
                    }
                    owners.insert(
                        run_id.clone(),
                        ApplicationSuspended::Runtime(Box::new(suspended)),
                    );
                    "acknowledgement_unknown"
                }
                SpawnStep::Conflict(_) => {
                    return Err(PublicError::BadRequest {
                        code: "AdmissionConflict",
                        message: "The run identity already has a different admission",
                    })
                }
                SpawnStep::Failed(_) => return Err(PublicError::Internal),
            };
            return Ok(AdmitRunResponse {
                run_id,
                disposition,
            });
        }
        let outcome = if entry_point_id.as_str() == EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID {
            let typed: EvmSubmissionRequest =
                serde_json::from_value(identity_input).map_err(|_| PublicError::Internal)?;
            let value = self
                .catalog
                .qualify(contract_ref, typed)
                .map_err(|_| PublicError::Internal)?;
            self.history
                .admit(run_id.clone(), &program, &value, configuration, Vec::new())
                .await
        } else {
            let typed: PortfolioSnapshotInput =
                serde_json::from_value(identity_input).map_err(|_| PublicError::Internal)?;
            let value = self
                .catalog
                .qualify(contract_ref, typed)
                .map_err(|_| PublicError::Internal)?;
            self.history
                .admit(run_id.clone(), &program, &value, configuration, Vec::new())
                .await
        };
        let disposition = match outcome {
            mfm_store::AdmissionOutcome::Selected(_, disposition) => match disposition {
                mfm_store::AppendDisposition::NewlyCommitted { .. } => "newly_committed",
                mfm_store::AppendDisposition::Found { .. } => "found",
                mfm_store::AppendDisposition::StaleHead { .. } => "stale_head",
                mfm_store::AppendDisposition::AcknowledgementUnknown => "acknowledgement_unknown",
            },
            mfm_store::AdmissionOutcome::AcknowledgementUnknown(owner)
            | mfm_store::AdmissionOutcome::RetainedRejected { owner, .. } => {
                self.suspended
                    .lock()
                    .map_err(|_| PublicError::Internal)?
                    .insert(
                        run_id.clone(),
                        ApplicationSuspended::Admission(Box::new(owner)),
                    );
                "acknowledgement_unknown"
            }
            mfm_store::AdmissionOutcome::Conflict(_) => {
                return Err(PublicError::BadRequest {
                    code: "AdmissionConflict",
                    message: "The run identity already has a different admission",
                })
            }
            mfm_store::AdmissionOutcome::Rejected(error) => return Err(map_store_error(error)),
        };
        Ok(AdmitRunResponse {
            run_id,
            disposition,
        })
    }

    /// Advances only callback-free retained state in this fixed facade.
    pub async fn drive(&self, run_id: RunId) -> Result<DriveResponse> {
        let run = self.reader.load(&run_id).await.map_err(map_store_error)?;
        let entry_point = match run.frames().first().map(RunFrame::record) {
            Some(RunRecord::RunAdmitted(admission)) => admission.entry_point_id().clone(),
            _ => return Err(PublicError::Internal),
        };
        if let Some(runtime) = self.runtimes.get(&entry_point) {
            return self.drive_with_runtime(runtime, run_id).await;
        }
        let document = self.document_for_run(&run)?;
        let program = self
            .catalog
            .program(document)
            .map_err(|_| PublicError::Internal)?;
        let selected = self
            .history
            .select(&run_id, &program)
            .await
            .map_err(map_store_error)?;
        if matches!(
            selected.action(),
            RunAction::ReadyPure { .. }
                | RunAction::ReadyAccess { .. }
                | RunAction::WaitingPreparation { .. }
        ) {
            return Err(PublicError::Internal);
        }
        Ok(DriveResponse {
            run_id,
            head_sequence: selected.head_sequence(),
            status: status_from_action(selected.action()),
        })
    }

    /// Resolves one application-retained suspended Runtime owner.
    pub async fn resolve_suspended(&self) -> Result<DriveResponse> {
        let suspended = {
            let mut owners = self.suspended.lock().map_err(|_| PublicError::Internal)?;
            if owners.len() != 1 {
                return Err(PublicError::Internal);
            }
            owners.pop_first().map(|(_, suspended)| suspended)
        }
        .ok_or(PublicError::Internal)?;
        self.resolve_application_suspended(suspended).await
    }

    /// Resolves one application-retained suspended Runtime owner by run identity.
    pub async fn resolve_suspended_run(&self, run_id: RunId) -> Result<DriveResponse> {
        let suspended = self
            .suspended
            .lock()
            .map_err(|_| PublicError::Internal)?
            .remove(&run_id)
            .ok_or(PublicError::Internal)?;
        self.resolve_application_suspended(suspended).await
    }

    async fn resolve_application_suspended(
        &self,
        suspended: ApplicationSuspended,
    ) -> Result<DriveResponse> {
        match suspended {
            ApplicationSuspended::Runtime(owner) => {
                self.finish_runtime_step((*owner).resolve().await).await
            }
            ApplicationSuspended::Admission(owner) => {
                let run_id = owner.run_id().clone();
                match self.history.resolve_admission(*owner).await {
                    mfm_store::AdmissionOutcome::Selected(selected, _) => Ok(DriveResponse {
                        run_id,
                        head_sequence: selected.head_sequence(),
                        status: status_from_action(selected.action()),
                    }),
                    mfm_store::AdmissionOutcome::AcknowledgementUnknown(owner)
                    | mfm_store::AdmissionOutcome::RetainedRejected { owner, .. } => {
                        self.suspended
                            .lock()
                            .map_err(|_| PublicError::Internal)?
                            .insert(run_id, ApplicationSuspended::Admission(Box::new(owner)));
                        Err(PublicError::Internal)
                    }
                    mfm_store::AdmissionOutcome::Conflict(_)
                    | mfm_store::AdmissionOutcome::Rejected(_) => Err(PublicError::Internal),
                }
            }
        }
    }

    async fn drive_with_runtime(&self, runtime: &Runtime, run_id: RunId) -> Result<DriveResponse> {
        match runtime.resume_run(run_id).await {
            ResumeStep::Active(session) => self.finish_runtime_step(session.drive().await).await,
            ResumeStep::Terminal(terminal) => Ok(DriveResponse {
                run_id: terminal.run_id().clone(),
                head_sequence: terminal.head_sequence(),
                status: RunStatus::Terminal,
            }),
            ResumeStep::Parked(parked) => Ok(DriveResponse {
                run_id: parked.run_id().clone(),
                head_sequence: parked.head_sequence(),
                status: RunStatus::WaitingPreparation,
            }),
            ResumeStep::Failed(_) => Err(PublicError::Internal),
        }
    }

    async fn finish_runtime_step(&self, step: RuntimeStep) -> Result<DriveResponse> {
        match step {
            RuntimeStep::Advanced(session) => {
                let run_id = session.run_id().clone();
                Ok(DriveResponse {
                    run_id,
                    head_sequence: session.head_sequence(),
                    status: RunStatus::Ready,
                })
            }
            RuntimeStep::Terminal(terminal) => Ok(DriveResponse {
                run_id: terminal.run_id().clone(),
                head_sequence: terminal.head_sequence(),
                status: RunStatus::Terminal,
            }),
            RuntimeStep::PreparationRejected { session, .. } => Ok(DriveResponse {
                run_id: session.run_id().clone(),
                head_sequence: session.head_sequence(),
                status: RunStatus::Ready,
            }),
            RuntimeStep::Unresolved { session, .. } => Ok(DriveResponse {
                run_id: session.run_id().clone(),
                head_sequence: session.head_sequence(),
                status: RunStatus::WaitingPreparation,
            }),
            RuntimeStep::Parked { session, .. } => Ok(DriveResponse {
                run_id: session.run_id().clone(),
                head_sequence: session.head_sequence(),
                status: RunStatus::WaitingPreparation,
            }),
            RuntimeStep::Suspended(suspended) => {
                let run_id = suspended.run_id().clone();
                {
                    let mut owners = self.suspended.lock().map_err(|_| PublicError::Internal)?;
                    if owners.contains_key(&run_id) {
                        return Err(PublicError::Internal);
                    }
                    owners.insert(
                        run_id.clone(),
                        ApplicationSuspended::Runtime(Box::new(suspended)),
                    );
                }
                let head_sequence = self
                    .reader
                    .load(&run_id)
                    .await
                    .map_err(map_store_error)?
                    .head_sequence();
                Ok(DriveResponse {
                    run_id,
                    head_sequence,
                    status: RunStatus::WaitingPreparation,
                })
            }
            RuntimeStep::AdmissionRejected(_)
            | RuntimeStep::ConclusionRejected { .. }
            | RuntimeStep::Conflict { .. }
            | RuntimeStep::Failed { .. } => Err(PublicError::Internal),
        }
    }

    /// Reads one run inside this fixed tenant partition.
    pub async fn read_public_run(&self, run_id: RunId) -> Result<PublicRunView> {
        let run = self.reader.load(&run_id).await.map_err(map_store_error)?;
        let document = self.document_for_run(&run)?;
        let program = self
            .catalog
            .program(document)
            .map_err(|_| PublicError::Internal)?;
        let selected = self
            .history
            .select(&run_id, &program)
            .await
            .map_err(map_store_error)?;
        Ok(PublicRunView {
            run_id,
            tenant_scope_id: self.tenant_scope_id.clone(),
            head_sequence: selected.head_sequence(),
            status: status_from_action(selected.action()),
        })
    }

    /// Replays a retained prefix with zero live callbacks.
    pub async fn replay_run(&self, run_id: RunId) -> Result<ReplayResponse> {
        let run = self.reader.load(&run_id).await.map_err(map_store_error)?;
        let document = self.document_for_run(&run)?;
        qualify_with_program(&run, document)
            .map(Into::into)
            .map_err(map_replay_error)
    }

    /// Returns a redacted structural trace without exposing retained values or objects.
    pub async fn trace_run(&self, run_id: RunId) -> Result<TraceResponse> {
        let run = self.reader.load(&run_id).await.map_err(map_store_error)?;
        let records = run
            .frames()
            .iter()
            .map(|frame| match frame.record() {
                RunRecord::RunAdmitted(_) => TraceRecord::Admitted,
                RunRecord::StatePrepared(prepared) => TraceRecord::Prepared {
                    occurrence: prepared.occurrence().clone(),
                    preparation_ordinal: prepared.preparation_ordinal(),
                },
                RunRecord::StateConcluded(conclusion) => TraceRecord::Concluded {
                    occurrence: conclusion.occurrence().clone(),
                    success: matches!(
                        conclusion.outcome(),
                        mfm_journal::single_trust::StateOutcome::Success(_)
                    ),
                },
            })
            .collect();
        Ok(TraceResponse {
            run_id,
            head_sequence: run.head_sequence(),
            records,
        })
    }

    /// Returns preparation/supersession/conclusion status without live callbacks.
    pub async fn audit_access(&self, run_id: RunId) -> Result<AccessAuditResponse> {
        let run = self.reader.load(&run_id).await.map_err(map_store_error)?;
        let mut entries: BTreeMap<SequentialControlAddress, AccessAuditEntry> = BTreeMap::new();
        for frame in run.frames() {
            match frame.record() {
                RunRecord::StatePrepared(prepared) => {
                    let entry = entries
                        .entry(prepared.occurrence().clone())
                        .or_insert_with(|| AccessAuditEntry {
                            occurrence: prepared.occurrence().clone(),
                            preparation_count: 0,
                            latest_preparation_ordinal: None,
                            concluded: false,
                        });
                    entry.preparation_count = entry.preparation_count.saturating_add(1);
                    entry.latest_preparation_ordinal = Some(prepared.preparation_ordinal());
                }
                RunRecord::StateConcluded(conclusion) => {
                    if let Some(entry) = entries.get_mut(conclusion.occurrence()) {
                        entry.concluded = true;
                    }
                }
                RunRecord::RunAdmitted(_) => {}
            }
        }
        Ok(AccessAuditResponse {
            run_id,
            head_sequence: run.head_sequence(),
            entries: entries.into_values().collect(),
        })
    }

    /// Exports the strict three-family frame stream.
    pub async fn export_run(&self, run_id: RunId) -> Result<ExportedRun> {
        let run = self.reader.load(&run_id).await.map_err(map_store_error)?;
        let portable = PortableRun::from_run(&run);
        let bytes = portable.encode().map_err(map_replay_error)?;
        Ok(ExportedRun { bytes })
    }

    fn document_for_run(&self, run: &mfm_store::QualifiedRun) -> Result<ProgramDocument> {
        let admission = match run.frames().first().map(RunFrame::record) {
            Some(RunRecord::RunAdmitted(admission)) => admission,
            Some(RunRecord::StatePrepared(_)) | Some(RunRecord::StateConcluded(_)) | None => {
                return Err(PublicError::Internal)
            }
        };
        if !self
            .supported_entry_points
            .contains_key(admission.entry_point_id())
        {
            return Err(PublicError::Internal);
        }
        let object = run
            .frames()
            .iter()
            .flat_map(|frame| frame.objects())
            .find(|object| object.content_ref() == admission.admitted_context().value_ref())
            .ok_or(PublicError::Internal)?;
        let input =
            serde_json::from_str(object.canonical_json()).map_err(|_| PublicError::Internal)?;
        let (_, _, document) = canonical_typed_admission(admission.entry_point_id(), input)
            .map_err(|_| PublicError::Internal)?;
        if program_ref(&document)? != *admission.program_ref() {
            return Err(PublicError::Internal);
        }
        self.catalog
            .program(document.clone())
            .map_err(|_| PublicError::Internal)?;
        Ok(document)
    }
}

fn canonical_typed_admission(
    entry_point_id: &StableId,
    input: serde_json::Value,
) -> Result<(PlainCanonicalJsonBytes, ContentRef, ProgramDocument)> {
    if entry_point_id.as_str() == PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID {
        let typed: PortfolioSnapshotInput =
            serde_json::from_value(input).map_err(|_| PublicError::BadRequest {
                code: "AdmissionRequestInvalid",
                message: "Portfolio admission does not match its typed contract",
            })?;
        typed.validate().map_err(|_| PublicError::BadRequest {
            code: "AdmissionRequestInvalid",
            message: "Portfolio admission violates its bounded domain contract",
        })?;
        let (canonical, contract) = canonical_typed_value(&typed)?;
        return Ok((canonical, contract, portfolio_program(&typed)?));
    }
    if entry_point_id.as_str() == EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID {
        let typed: EvmSubmissionRequest =
            serde_json::from_value(input).map_err(|_| PublicError::BadRequest {
                code: "AdmissionRequestInvalid",
                message: "EVM admission does not match its typed contract",
            })?;
        typed.validate().map_err(|_| PublicError::BadRequest {
            code: "AdmissionRequestInvalid",
            message: "EVM admission violates its bounded domain contract",
        })?;
        let (canonical, contract) = canonical_typed_value(&typed)?;
        return Ok((canonical, contract, evm_submission_program()?));
    }
    Err(PublicError::BadRequest {
        code: "EntryPointNotFound",
        message: "The entry point is not registered",
    })
}

fn canonical_typed_value<T: MfmValue>(value: &T) -> Result<(PlainCanonicalJsonBytes, ContentRef)> {
    let canonical = mfm_program::canonical_value(value).map_err(|_| PublicError::Internal)?;
    let contract = nominal_contract_ref::<T>().map_err(|_| PublicError::Internal)?;
    Ok((canonical, contract))
}

/// Builds the exact catalog used by trusted application composition.
pub fn application_catalog() -> Result<ProgramCatalog> {
    application_catalog_builder()?
        .finish(evm_submission_program()?)
        .map(|(catalog, _)| catalog)
        .map_err(|_| PublicError::Internal)
}

fn application_catalog_builder() -> Result<ProgramCatalogBuilder> {
    let mut builder = ProgramCatalog::builder();
    builder
        .register_value::<EvmSubmissionRequest>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<EvmSubmissionContext>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<EvmSubmissionOutput>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<EvmSubmissionFailure>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<PortfolioSnapshotInput>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<PortfolioContinuation>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<EvmBalanceContext<PortfolioContinuation>>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<EvmBalanceAsset>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<EvmNativeBalanceInput>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<EvmTokenBalanceInput>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<EvmBalanceCollectionCompletion<PortfolioContinuation>>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<PortfolioSnapshotOutput>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<PortfolioSnapshotFailure>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<EvmReadIntent>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<EvmReadEvidence>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<BroadcastIntent>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_value::<BroadcastEvidence>()
        .map_err(|_| PublicError::Internal)?;
    Ok(builder)
}

fn named_ref(schema_name: &str, identity: &str) -> Result<ContentRef> {
    value_ref(schema_name, identity.as_bytes())
}

fn implementation_ref(identity: &str) -> Result<ContentRef> {
    named_ref("mfm.state-implementation", identity)
}

fn binding_ref(identity: &str) -> Result<ContentRef> {
    named_ref("mfm.execution-binding", identity)
}

fn capability_ref<C: AccessCapabilityContract>() -> Result<ContentRef> {
    let identity = C::contract_id().map_err(|_| PublicError::Internal)?;
    named_ref("mfm.capability-contract", identity.as_str())
}

fn address(ordinal: usize) -> Result<SequentialControlAddress> {
    SequentialControlAddress::new(
        u32::try_from(ordinal).map_err(|_| PublicError::Internal)?,
        Vec::new(),
    )
    .map_err(|_| PublicError::Internal)
}

fn pure_state(
    ordinal: usize,
    input: ContentRef,
    output: ContentRef,
    failure: Option<ContentRef>,
    next: Option<SequentialControlAddress>,
    failure_next: Option<SequentialControlAddress>,
    implementation_name: &str,
) -> Result<StateDeclaration> {
    let state_address = address(ordinal)?;
    let implementation = implementation_ref(implementation_name)?;
    let mut state = match next {
        Some(next) => StateDeclaration::with_next(
            state_address,
            implementation,
            input,
            output,
            failure,
            ExecutionMode::Pure,
            next,
        ),
        None => StateDeclaration::new(
            state_address,
            implementation,
            input,
            output,
            failure,
            ExecutionMode::Pure,
            true,
        ),
    }
    .map_err(|_| PublicError::Internal)?;
    if let Some(failure_next) = failure_next {
        state = state
            .with_failure_next(failure_next)
            .map_err(|_| PublicError::Internal)?;
    }
    Ok(state)
}

#[allow(clippy::too_many_arguments)]
fn read_state_to(
    ordinal: usize,
    input: ContentRef,
    output: ContentRef,
    failure: ContentRef,
    failure_next: SequentialControlAddress,
    next: Option<SequentialControlAddress>,
    capability_contract_ref: ContentRef,
    implementation_name: &str,
) -> Result<StateDeclaration> {
    let state_address = address(ordinal)?;
    let implementation = implementation_ref(implementation_name)?;
    let binding = binding_ref(implementation_name)?;
    let execution = ExecutionMode::Read {
        capability_contract_ref,
        total_attempt_bound: 3,
        fact_selection_required: false,
    };
    let mut state = match next {
        Some(next) => StateDeclaration::with_next(
            state_address,
            implementation,
            input,
            output,
            Some(failure),
            execution,
            next,
        ),
        None => StateDeclaration::new(
            state_address,
            implementation,
            input,
            output,
            Some(failure),
            execution,
            true,
        ),
    }
    .map_err(|_| PublicError::Internal)?
    .with_execution_binding(binding)
    .map_err(|_| PublicError::Internal)?;
    state = state
        .with_failure_next(failure_next)
        .map_err(|_| PublicError::Internal)?;
    Ok(state)
}

fn effect_state_to(
    ordinal: usize,
    input: ContentRef,
    output: ContentRef,
    failure: ContentRef,
    failure_next: SequentialControlAddress,
    capability_contract_ref: ContentRef,
    implementation_name: &str,
) -> Result<StateDeclaration> {
    let state_address = address(ordinal)?;
    let implementation = implementation_ref(implementation_name)?;
    let binding = binding_ref(implementation_name)?;
    let effect_domain = StableId::new(implementation_name).map_err(|_| PublicError::Internal)?;
    let state = StateDeclaration::new(
        state_address,
        implementation,
        input,
        output,
        Some(failure),
        ExecutionMode::Effect {
            capability_contract_ref,
            effect_domain,
            fact_selection_required: false,
        },
        true,
    )
    .map_err(|_| PublicError::Internal)?
    .with_execution_binding(binding)
    .map_err(|_| PublicError::Internal)?
    .with_failure_next(failure_next)
    .map_err(|_| PublicError::Internal)?;
    Ok(state)
}

fn evm_submission_program() -> Result<ProgramDocument> {
    let entry_point_id =
        StableId::new(EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID).map_err(|_| PublicError::Internal)?;
    let request =
        nominal_contract_ref::<EvmSubmissionRequest>().map_err(|_| PublicError::Internal)?;
    let context =
        nominal_contract_ref::<EvmSubmissionContext>().map_err(|_| PublicError::Internal)?;
    let output =
        nominal_contract_ref::<EvmSubmissionOutput>().map_err(|_| PublicError::Internal)?;
    let failure =
        nominal_contract_ref::<EvmSubmissionFailure>().map_err(|_| PublicError::Internal)?;
    let failure_address = address(4)?;
    let read = read_state_to(
        1,
        request.clone(),
        context.clone(),
        failure.clone(),
        failure_address.clone(),
        Some(address(2)?),
        capability_ref::<ReadWalletNonceStatus>()?,
        "mfm.evm.state.read-wallet-nonce@1",
    )?;
    let derive = pure_state(
        2,
        context.clone(),
        context.clone(),
        None,
        Some(address(3)?),
        None,
        "mfm.evm.state.derive-candidate@1",
    )?;
    let broadcast = effect_state_to(
        3,
        context.clone(),
        output.clone(),
        failure.clone(),
        failure_address.clone(),
        capability_ref::<BroadcastTransaction>()?,
        "mfm.evm.state.broadcast@1",
    )?;
    let failure_state = pure_state(
        4,
        failure.clone(),
        output.clone(),
        Some(output.clone()),
        None,
        None,
        "mfm.evm.state.failure@1",
    )?;
    ProgramDocument::new(
        entry_point_id,
        output,
        request,
        vec![
            Declaration::State(Box::new(read)),
            Declaration::State(Box::new(derive)),
            Declaration::State(Box::new(broadcast)),
            Declaration::State(Box::new(failure_state)),
        ],
    )
    .map_err(|_| PublicError::Internal)
}

struct PortfolioSourcePlan {
    select: SequentialControlAddress,
    selector: SequentialControlAddress,
    native: Vec<SequentialControlAddress>,
    token: Vec<SequentialControlAddress>,
    after: SequentialControlAddress,
}

struct PortfolioCollectionPlan {
    entry: SequentialControlAddress,
    sources: Vec<PortfolioSourcePlan>,
    consolidate: SequentialControlAddress,
    resume: SequentialControlAddress,
}

fn portfolio_program(input: &PortfolioSnapshotInput) -> Result<ProgramDocument> {
    let entry_point_id =
        StableId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).map_err(|_| PublicError::Internal)?;
    let admitted =
        nominal_contract_ref::<PortfolioSnapshotInput>().map_err(|_| PublicError::Internal)?;
    let continuation =
        nominal_contract_ref::<PortfolioContinuation>().map_err(|_| PublicError::Internal)?;
    let balance_context = nominal_contract_ref::<EvmBalanceContext<PortfolioContinuation>>()
        .map_err(|_| PublicError::Internal)?;
    let balance_asset =
        nominal_contract_ref::<EvmBalanceAsset>().map_err(|_| PublicError::Internal)?;
    let native_input =
        nominal_contract_ref::<EvmNativeBalanceInput>().map_err(|_| PublicError::Internal)?;
    let token_input =
        nominal_contract_ref::<EvmTokenBalanceInput>().map_err(|_| PublicError::Internal)?;
    let balance_completion =
        nominal_contract_ref::<EvmBalanceCollectionCompletion<PortfolioContinuation>>()
            .map_err(|_| PublicError::Internal)?;
    let output =
        nominal_contract_ref::<PortfolioSnapshotOutput>().map_err(|_| PublicError::Internal)?;
    let failure =
        nominal_contract_ref::<PortfolioSnapshotFailure>().map_err(|_| PublicError::Internal)?;

    let mut next_ordinal = 1usize;
    let mut allocate = || -> Result<SequentialControlAddress> {
        let ordinal = next_ordinal;
        next_ordinal = next_ordinal.checked_add(1).ok_or(PublicError::Internal)?;
        address(ordinal)
    };
    let initial = allocate()?;
    let mut plans = Vec::with_capacity(input.collections.len());
    for request in &input.collections {
        let entry = allocate()?;
        let mut sources = Vec::with_capacity(request.sources.len());
        for _ in &request.sources {
            let select = allocate()?;
            let selector = allocate()?;
            let native = (0..4).map(|_| allocate()).collect::<Result<Vec<_>>>()?;
            let token = (0..5).map(|_| allocate()).collect::<Result<Vec<_>>>()?;
            let after = allocate()?;
            sources.push(PortfolioSourcePlan {
                select,
                selector,
                native,
                token,
                after,
            });
        }
        plans.push(PortfolioCollectionPlan {
            entry,
            sources,
            consolidate: allocate()?,
            resume: allocate()?,
        });
    }
    let final_address = allocate()?;
    let failure_address = allocate()?;
    let mut declarations = Vec::new();

    declarations.push(Declaration::State(Box::new(
        StateDeclaration::with_next(
            initial,
            implementation_ref("mfm.portfolio.state.initialize@1")?,
            admitted,
            continuation.clone(),
            None,
            ExecutionMode::Pure,
            plans[0].entry.clone(),
        )
        .map_err(|_| PublicError::Internal)?,
    )));

    for (collection_ordinal, (request, plan)) in input.collections.iter().zip(&plans).enumerate() {
        declarations.push(Declaration::State(Box::new(
            StateDeclaration::with_next(
                plan.entry.clone(),
                implementation_ref(&format!(
                    "mfm.portfolio.state.enter-collection-{}@1",
                    collection_ordinal
                ))?,
                continuation.clone(),
                balance_context.clone(),
                None,
                ExecutionMode::Pure,
                plan.sources[0].select.clone(),
            )
            .map_err(|_| PublicError::Internal)?,
        )));

        for (source_ordinal, (_source, source_plan)) in
            request.sources.iter().zip(&plan.sources).enumerate()
        {
            declarations.push(Declaration::State(Box::new(pure_state(
                source_plan.select.declaration_ordinal() as usize,
                balance_context.clone(),
                balance_asset.clone(),
                None,
                Some(source_plan.selector.clone()),
                None,
                &format!(
                    "mfm.evm.state.collection-{}-source-{}-select-asset@1",
                    collection_ordinal, source_ordinal
                ),
            )?)));
            declarations.push(Declaration::Match(
                MatchDeclaration::new(
                    source_plan.selector.clone(),
                    balance_asset.clone(),
                    vec![
                        MatchVariant::new(
                            StableId::new("native").map_err(|_| PublicError::Internal)?,
                            native_input.clone(),
                            balance_context.clone(),
                            source_plan.native[0].clone(),
                        ),
                        MatchVariant::new(
                            StableId::new("token").map_err(|_| PublicError::Internal)?,
                            token_input.clone(),
                            balance_context.clone(),
                            source_plan.token[0].clone(),
                        ),
                    ],
                )
                .map_err(|_| PublicError::Internal)?,
            ));

            for (stage, stage_address) in source_plan.native.iter().enumerate() {
                let input_contract = if stage == 0 {
                    native_input.clone()
                } else {
                    balance_context.clone()
                };
                let next = if stage + 1 < source_plan.native.len() {
                    source_plan.native[stage + 1].clone()
                } else {
                    source_plan.after.clone()
                };
                declarations.push(Declaration::State(Box::new(read_state_to(
                    stage_address.declaration_ordinal() as usize,
                    input_contract,
                    balance_context.clone(),
                    failure.clone(),
                    failure_address.clone(),
                    Some(next),
                    capability_ref::<ReadBalance>()?,
                    &format!(
                        "mfm.evm.state.collection-{}-source-{}-native-stage-{}@1",
                        collection_ordinal, source_ordinal, stage
                    ),
                )?)));
            }
            for (stage, stage_address) in source_plan.token.iter().enumerate() {
                let input_contract = if stage == 0 {
                    token_input.clone()
                } else {
                    balance_context.clone()
                };
                let next = if stage + 1 < source_plan.token.len() {
                    source_plan.token[stage + 1].clone()
                } else {
                    source_plan.after.clone()
                };
                declarations.push(Declaration::State(Box::new(read_state_to(
                    stage_address.declaration_ordinal() as usize,
                    input_contract,
                    balance_context.clone(),
                    failure.clone(),
                    failure_address.clone(),
                    Some(next),
                    capability_ref::<ReadBalance>()?,
                    &format!(
                        "mfm.evm.state.collection-{}-source-{}-token-stage-{}@1",
                        collection_ordinal, source_ordinal, stage
                    ),
                )?)));
            }
            let next = if source_ordinal + 1 < plan.sources.len() {
                plan.sources[source_ordinal + 1].select.clone()
            } else {
                plan.consolidate.clone()
            };
            declarations.push(Declaration::State(Box::new(pure_state(
                source_plan.after.declaration_ordinal() as usize,
                balance_context.clone(),
                balance_context.clone(),
                None,
                Some(next),
                None,
                &format!(
                    "mfm.evm.state.collection-{}-source-{}-complete@1",
                    collection_ordinal, source_ordinal
                ),
            )?)));
        }

        declarations.push(Declaration::State(Box::new(pure_state(
            plan.consolidate.declaration_ordinal() as usize,
            balance_context.clone(),
            balance_completion.clone(),
            None,
            Some(plan.resume.clone()),
            None,
            &format!(
                "mfm.evm.state.consolidate-collection-{}@1",
                collection_ordinal
            ),
        )?)));
        let resume_next = if collection_ordinal + 1 < plans.len() {
            plans[collection_ordinal + 1].entry.clone()
        } else {
            final_address.clone()
        };
        declarations.push(Declaration::State(Box::new(pure_state(
            plan.resume.declaration_ordinal() as usize,
            balance_completion.clone(),
            continuation.clone(),
            None,
            Some(resume_next),
            None,
            &format!(
                "mfm.portfolio.state.resume-collection-{}@1",
                collection_ordinal
            ),
        )?)));
    }

    declarations.push(Declaration::State(Box::new(pure_state(
        final_address.declaration_ordinal() as usize,
        continuation.clone(),
        output.clone(),
        Some(failure.clone()),
        None,
        Some(failure_address.clone()),
        "mfm.portfolio.state.consolidate@1",
    )?)));
    declarations.push(Declaration::State(Box::new(pure_state(
        failure_address.declaration_ordinal() as usize,
        failure,
        output.clone(),
        Some(output),
        None,
        None,
        "mfm.portfolio.state.failure@1",
    )?)));
    ProgramDocument::new(
        entry_point_id,
        nominal_contract_ref::<PortfolioSnapshotOutput>().map_err(|_| PublicError::Internal)?,
        nominal_contract_ref::<PortfolioSnapshotInput>().map_err(|_| PublicError::Internal)?,
        declarations,
    )
    .map_err(|_| PublicError::Internal)
}

fn program_ref(document: &ProgramDocument) -> Result<ContentRef> {
    document.program_ref().map_err(|_| PublicError::Internal)
}

fn status_from_action(action: &RunAction) -> RunStatus {
    match action {
        RunAction::ZeroStateTerminal { .. } | RunAction::Terminal { .. } => RunStatus::Terminal,
        RunAction::Failed { .. } => RunStatus::Failed,
        RunAction::ReadyPure { .. } | RunAction::ReadyAccess { .. } => RunStatus::Ready,
        RunAction::WaitingPreparation { .. } => RunStatus::WaitingPreparation,
    }
}

fn canonical_json(value: &serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let encoded = serde_json::to_string(value).map_err(|_| PublicError::Internal)?;
    PlainCanonicalJsonBytes::from_json_str(&encoded).map_err(|_| PublicError::Internal)
}

fn value_ref(schema_name: &str, bytes: &[u8]) -> Result<ContentRef> {
    let schema = SchemaId::new(
        schema_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| PublicError::Internal)?;
    ContentRef::new(schema, raw_content_digest(bytes)).map_err(|_| PublicError::Internal)
}

fn map_store_error(error: StoreError) -> PublicError {
    match error {
        StoreError::NotFound => PublicError::RunNotFound,
        StoreError::Identity
        | StoreError::InvalidRecord
        | StoreError::Conflict
        | StoreError::Capacity
        | StoreError::NotActionable
        | StoreError::InvalidHistory
        | StoreError::FactFrontierChanged => PublicError::Internal,
    }
}

fn map_replay_error(_: ReplayError) -> PublicError {
    PublicError::Internal
}

fn contains_float(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(number) => number.is_f64(),
        serde_json::Value::Array(values) => values.iter().any(contains_float),
        serde_json::Value::Object(values) => values.values().any(contains_float),
        _ => false,
    }
}

fn contains_secret_marker(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => string_contains_secret_marker(text),
        serde_json::Value::Array(values) => values.iter().any(contains_secret_marker),
        serde_json::Value::Object(values) => values.iter().any(|(key, value)| {
            contains_secret_marker(&serde_json::Value::String(key.clone()))
                || contains_secret_marker(value)
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_evm::{
        EvmBalanceCollectionResult, EvmBalanceResult, EvmTransactionTarget,
        EVM_TRANSACTION_DATA_LIMIT,
    };
    use mfm_program::single_trust::ProgramIngress;
    use mfm_store::ConfigurationCommitOutcome;
    use mfm_values::ValidatedConfig;

    async fn test_components() -> (
        QualifiedHistoryPort,
        HistoryReader,
        ConfigurationStore,
        StoreAuditPort,
        ResolvedConfigurationHead,
        ResolvedConfigurationHead,
    ) {
        let identity = StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            StoreEpoch::new(1),
            TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        );
        let store = StructuredStore::open_memory(
            identity,
            application_catalog().expect("catalog"),
            StoreWorkLimits::default(),
        )
        .expect("store");
        let (history, reader, configuration, audit) = store.split().into_parts();
        let portfolio = configuration
            .initial_write_session::<PortfolioConfig>()
            .prepare_local(
                AppendRequestId::new("test-portfolio-configuration-0001").expect("request"),
                ValidatedConfig::new(PortfolioConfig {
                    portfolio_id: mfm_portfolio::PortfolioId {
                        value: "portfolio-1".to_owned(),
                    },
                    quotes: vec![mfm_portfolio::QuoteCode::Usd],
                })
                .expect("valid portfolio config"),
            )
            .expect("portfolio append");
        let portfolio = match configuration.commit(portfolio).await.expect("commit") {
            ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
            _ => panic!("unexpected portfolio configuration outcome"),
        };
        let portfolio_head = portfolio.head().clone();
        let evm = configuration
            .write_session::<EvmConfig>(&portfolio_head)
            .expect("evm session")
            .prepare_local(
                AppendRequestId::new("test-evm-configuration-000000001").expect("request"),
                ValidatedConfig::new(EvmConfig {}).expect("valid evm config"),
            )
            .expect("evm append");
        let evm = match configuration.commit(evm).await.expect("commit") {
            ConfigurationCommitOutcome::NewlyCommitted(resolved) => resolved,
            _ => panic!("unexpected evm configuration outcome"),
        };
        let evm_head = evm.into_head();
        let restarted_portfolio = configuration
            .load::<PortfolioConfig>()
            .await
            .expect("restarted Portfolio config");
        let restarted_evm = configuration
            .load::<EvmConfig>()
            .await
            .expect("restarted EVM config");
        assert_eq!(
            restarted_portfolio.head().sequence(),
            portfolio_head.sequence()
        );
        assert_eq!(restarted_evm.head().sequence(), evm_head.sequence());
        (
            history,
            reader,
            configuration,
            audit,
            restarted_portfolio.into_head(),
            restarted_evm.into_head(),
        )
    }

    async fn test_application() -> Application {
        let (history, reader, configuration, audit, portfolio, evm) = test_components().await;
        Application::new(
            history,
            reader,
            configuration,
            audit,
            portfolio,
            evm,
            Vec::new(),
        )
        .expect("application")
    }

    #[tokio::test]
    async fn admits_typed_input_into_sequential_program() {
        let app = test_application().await;
        let entry = StableId::new(EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID).expect("entry");
        let input = serde_json::json!({
            "target": {"chain_id": 1, "sender": "0xabc", "nonce_domain": "wallet-main"},
            "idempotency_key": "op-1",
            "data": [],
            "gas_limit": 21000,
            "max_fee": "100"
        });
        let response = app
            .admit_run(AdmitRunRequest::new(entry.clone(), input).expect("request"))
            .await;
        let response = response.expect("admission");
        let retained = app.reader.load(&response.run_id).await.expect("retained");
        let RunRecord::RunAdmitted(admitted) = retained.frames()[0].record() else {
            panic!("expected admission")
        };
        let configured = app
            .configuration_heads
            .get(&entry)
            .expect("configured EVM head");
        assert_eq!(admitted.configuration().sequence(), configured.sequence());
        assert_eq!(
            admitted.configuration().content_ref(),
            configured.content_ref()
        );
        assert_eq!(
            app.drive(response.run_id.clone()).await,
            Err(PublicError::Internal)
        );
    }

    #[tokio::test]
    async fn evm_submission_identity_is_tenant_nonce_domain_and_idempotency_key() {
        let app = test_application().await;
        let entry = StableId::new(EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID).expect("entry");
        let request = |max_fee: &str| {
            AdmitRunRequest::new(
                entry.clone(),
                serde_json::json!({
                    "target": {"chain_id": 1, "sender": "0xabc", "nonce_domain": "wallet-main"},
                    "idempotency_key": "op-1",
                    "data": [],
                    "gas_limit": 21000,
                    "max_fee": max_fee
                }),
            )
            .expect("request")
        };

        let first = app
            .admit_run(request("100"))
            .await
            .expect("first admission");
        let retry = app
            .admit_run(request("100"))
            .await
            .expect("retry admission");
        assert_eq!(retry.run_id, first.run_id);
        assert_eq!(retry.disposition, "found");
        assert_eq!(
            app.admit_run(request("101")).await,
            Err(PublicError::BadRequest {
                code: "AdmissionConflict",
                message: "The run identity already has a different admission",
            })
        );
    }

    #[tokio::test]
    async fn plans_portfolio_asset_match_for_each_collection() {
        let app = test_application().await;
        let entry = StableId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).expect("entry");
        let input = serde_json::json!({
            "portfolio_id": {"value": "portfolio-1"},
            "collections": [{
                "sources": [{
                    "source_id": "native-1",
                    "chain_id": 1,
                    "address": "0xabc",
                    "token": null
                }],
                "decimals": 18
            }],
            "quote": "usd"
        });
        let response = app
            .admit_run(AdmitRunRequest::new(entry, input).expect("request"))
            .await
            .expect("admission");
        let retained = app.reader.load(&response.run_id).await.expect("retained");
        let RunRecord::RunAdmitted(admitted) = retained.frames()[0].record() else {
            panic!("expected admission")
        };
        let portfolio_entry =
            StableId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).expect("portfolio entry");
        let configured = app
            .configuration_heads
            .get(&portfolio_entry)
            .expect("configured Portfolio head");
        assert_eq!(admitted.configuration().sequence(), configured.sequence());
        assert_eq!(
            admitted.configuration().content_ref(),
            configured.content_ref()
        );
        assert_eq!(app.drive(response.run_id).await, Err(PublicError::Internal));
    }

    #[tokio::test]
    async fn exports_and_reimports_the_v5_structural_stream() {
        let app = test_application().await;
        let entry = StableId::new(EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID).expect("entry");
        let input = serde_json::json!({
            "target": {"chain_id": 1, "sender": "0xabc", "nonce_domain": "wallet-main"},
            "idempotency_key": "op-1",
            "data": [],
            "gas_limit": 21000,
            "max_fee": "100"
        });
        let response = app
            .admit_run(AdmitRunRequest::new(entry, input).expect("request"))
            .await
            .expect("admission");
        let export = app.export_run(response.run_id).await.expect("export");
        let portable = PortableRun::decode(export.bytes()).expect("portable");
        let content_ref = portable.content_ref().expect("content ref");
        assert_eq!(
            PortableRun::decode_expected(export.bytes(), &content_ref).expect("expected"),
            portable
        );
        assert!(PortableRun::decode(br#"{"format":"mfm.portable-run.v4"}"#).is_err());
    }

    #[tokio::test]
    async fn composition_rejects_foreign_and_wrong_type_configuration_heads() {
        let (history, reader, configuration, audit, _, _) = test_components().await;
        let (_, _, _, _, second_portfolio, second_evm) = test_components().await;

        assert!(matches!(
            Application::new(
                history,
                reader,
                configuration,
                audit,
                second_portfolio,
                second_evm,
                Vec::new(),
            ),
            Err(PublicError::Internal)
        ));
        let (history, reader, configuration, audit, first_portfolio, first_evm) =
            test_components().await;
        assert!(matches!(
            Application::new(
                history,
                reader,
                configuration,
                audit,
                first_evm,
                first_portfolio,
                Vec::new(),
            ),
            Err(PublicError::Internal)
        ));
    }

    #[test]
    fn strict_admission_json_rejects_nested_duplicate_keys() {
        assert!(parse_admission_json(
            r#"{"entry_point_id":"mfm.test@1","input":{"value":1,"value":2}}"#
        )
        .is_err());
    }

    #[test]
    fn maximum_entry_point_programs_record_capacity_envelope() {
        let evm = evm_submission_program().expect("EVM program");
        let evm_bytes = evm.canonical_bytes().expect("EVM canonical bytes");
        let evm_input = EvmSubmissionRequest::new(
            EvmTransactionTarget::new(1, "0xabc".to_owned(), "wallet-main".to_owned())
                .expect("EVM target"),
            "capacity-envelope".to_owned(),
            vec![0; EVM_TRANSACTION_DATA_LIMIT],
            21_000,
            "100".to_owned(),
        )
        .expect("maximum EVM input");
        let evm_c0_bytes = mfm_program::canonical_value(&evm_input)
            .expect("EVM C0 bytes")
            .as_bytes()
            .len();
        let evm_context = EvmSubmissionContext::new(
            evm_input,
            u64::MAX,
            "capacity-envelope-candidate".to_owned(),
        )
        .expect("maximum EVM context");
        let evm_cn_bytes = mfm_program::canonical_value(&evm_context)
            .expect("EVM Cn bytes")
            .as_bytes()
            .len();

        let collection_count =
            mfm_portfolio::PORTFOLIO_COLLECTION_LIMIT.min(mfm_evm::EVM_BALANCE_SOURCE_LIMIT);
        let collections = (0..collection_count)
            .map(|collection| {
                serde_json::json!({
                    "sources": [{
                        "source_id": format!("source-{collection}"),
                        "chain_id": 1,
                        "address": format!("0x{collection:02x}"),
                        "token": null
                    }],
                    "decimals": 18
                })
            })
            .collect::<Vec<_>>();
        let input: PortfolioSnapshotInput = serde_json::from_value(serde_json::json!({
            "portfolio_id": {"value": "portfolio-capacity-envelope"},
            "collections": collections,
            "quote": "usd"
        }))
        .expect("maximum Portfolio input");
        let portfolio = portfolio_program(&input).expect("Portfolio program");
        let portfolio_bytes = portfolio
            .canonical_bytes()
            .expect("Portfolio canonical bytes");
        let portfolio_c0_bytes = mfm_program::canonical_value(&input)
            .expect("Portfolio C0 bytes")
            .as_bytes()
            .len();
        let portfolio_collections = input.collections.len();
        let portfolio_sources = input
            .collections
            .iter()
            .map(|collection| collection.sources.len())
            .sum::<usize>();
        let portfolio_declarations = portfolio.declarations().len();
        let portfolio_completed = input
            .collections
            .iter()
            .enumerate()
            .map(|(collection, request)| EvmBalanceCollectionResult {
                results: request
                    .sources
                    .iter()
                    .map(|source| EvmBalanceResult {
                        source_id: source.source_id.clone(),
                        amount_scaled: "0".to_owned(),
                        anchor: format!("anchor-{collection}"),
                    })
                    .collect(),
                total_scaled: "0".to_owned(),
            })
            .collect();
        let portfolio_context = PortfolioContinuation::new(input, portfolio_completed, 64)
            .expect("maximum Portfolio context");
        let portfolio_cn_bytes = mfm_program::canonical_value(&portfolio_context)
            .expect("Portfolio Cn bytes")
            .as_bytes()
            .len();

        let (evm_catalog, evm_program) = application_catalog_builder()
            .expect("catalog builder")
            .finish(evm)
            .expect("EVM Program finish");
        ProgramIngress::new(&evm_catalog)
            .decode(evm_bytes.as_bytes())
            .expect("EVM Program ingress");
        let (portfolio_catalog, _portfolio_program) = application_catalog_builder()
            .expect("catalog builder")
            .finish(portfolio)
            .expect("Portfolio Program finish");
        ProgramIngress::new(&portfolio_catalog)
            .decode(portfolio_bytes.as_bytes())
            .expect("Portfolio Program ingress");

        eprintln!(
            "capacity-envelope app evm declarations={} bytes={} c0_bytes={} cn_bytes={} portfolio collections={} sources={} declarations={} bytes={} c0_bytes={} cn_bytes={}",
            evm_program.document().declarations().len(),
            evm_bytes.as_bytes().len(),
            evm_c0_bytes,
            evm_cn_bytes,
            portfolio_collections,
            portfolio_sources,
            portfolio_declarations,
            portfolio_bytes.as_bytes().len(),
            portfolio_c0_bytes,
            portfolio_cn_bytes,
        );
    }
}
