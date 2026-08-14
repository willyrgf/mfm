#![warn(missing_docs)]
//! Fixed-tenant admission orchestration for the two supported domain entry points.
//!
//! App parses bounded public selectors, invokes the domain planners, qualifies their exact final
//! Program and `C0`, and delegates every mutation to one mandatory catalog-wide Runtime. It owns
//! no Program authoring, adapter identity construction, provider fallback, or domain transition.

use std::collections::BTreeMap;
use std::sync::Mutex;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_evm::{
    plan_submission, submission_closure_documents, EvmBalanceAsset, EvmBalanceBindings,
    EvmBalanceCollectionCompletion, EvmBalanceContext, EvmBalanceFailure, EvmCapability, EvmConfig,
    EvmSubmissionBindings, EvmSubmissionFailure, EvmSubmissionOutput, EvmSubmissionProgress,
    EvmSubmissionRequest, EvmSubmissionSelector, EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID,
};
use mfm_ids::{DigestAlgorithm, RunId, StableId, TenantScopeId};
use mfm_journal::single_trust::{RunFrame, RunRecord};
use mfm_portfolio::{
    plan_snapshot, snapshot_closure_document, PortfolioConfig, PortfolioContinuation,
    PortfolioSnapshotFailure, PortfolioSnapshotInput, PortfolioSnapshotOutput,
    PortfolioSnapshotSelector, PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
};
use mfm_program::{nominal_contract_ref, ProgramCatalog, ProgramCatalogBuilder, ProgramDocument};
use mfm_replay::{qualify_with_retained_program, PortableRun, ReplayReport};
use mfm_runtime::{ResumeStep, Runtime, RuntimeStep, SpawnStep, SuspendedRun};
use mfm_store::{
    ConfigurationStore, HistoryReader, ResolvedConfiguration, ResolvedConfigurationHead,
    StoreAuditPort,
};
use mfm_values::MfmValue;
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

/// Strict singular transport admission request.
#[derive(Debug, Clone)]
pub struct AdmitRunRequest {
    entry_point_id: StableId,
    selector: serde_json::Value,
}

impl AdmitRunRequest {
    /// Validates a request from an already parsed transport value.
    pub fn new(entry_point_id: StableId, selector: serde_json::Value) -> Result<Self> {
        validate_transport_value(&selector)?;
        Ok(Self {
            entry_point_id,
            selector,
        })
    }
}

/// Parses one selector JSON value after rejecting duplicate keys, floats, and oversized input.
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
        occurrence: mfm_program::SequentialControlAddress,
        /// Preparation attempt ordinal.
        preparation_ordinal: u16,
    },
    /// Durable State conclusion.
    Concluded {
        /// Exact sequential occurrence.
        occurrence: mfm_program::SequentialControlAddress,
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
    pub occurrence: mfm_program::SequentialControlAddress,
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

/// Fixed-tenant Application with one mandatory Runtime and finite exact planner bindings.
pub struct Application {
    tenant_scope_id: TenantScopeId,
    reader: HistoryReader,
    _configuration: ConfigurationStore,
    _audit: StoreAuditPort,
    runtime: Runtime,
    portfolio_configuration: PortfolioConfig,
    portfolio_configuration_head: ResolvedConfigurationHead,
    evm_configuration: EvmConfig,
    evm_configuration_head: ResolvedConfigurationHead,
    submission_bindings: Vec<EvmSubmissionBindings>,
    balance_bindings: Vec<EvmBalanceBindings>,
    suspended: Mutex<BTreeMap<RunId, Box<SuspendedRun>>>,
}

impl Application {
    /// Composes one complete fixed-tenant facade from a mandatory live Runtime and typed config.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        reader: HistoryReader,
        configuration: ConfigurationStore,
        audit: StoreAuditPort,
        runtime: Runtime,
        portfolio_configuration: ResolvedConfiguration<PortfolioConfig>,
        evm_configuration: ResolvedConfiguration<EvmConfig>,
        submission_bindings: Vec<EvmSubmissionBindings>,
        balance_bindings: Vec<EvmBalanceBindings>,
    ) -> Result<Self> {
        if reader.identity() != runtime.store_identity()
            || configuration.identity() != runtime.store_identity()
            || audit.identity() != runtime.store_identity()
            || !configuration.owns_head(portfolio_configuration.head())
            || !configuration.owns_head(evm_configuration.head())
            || !runtime.accepts_configuration_head(portfolio_configuration.head())
            || !runtime.accepts_configuration_head(evm_configuration.head())
            || submission_bindings.is_empty()
            || balance_bindings.is_empty()
        {
            return Err(PublicError::Internal);
        }
        let portfolio_value = portfolio_configuration.value().clone();
        let portfolio_head = portfolio_configuration.into_head();
        let evm_value = evm_configuration.value().clone();
        let evm_head = evm_configuration.into_head();
        validate_runtime_closure(
            &runtime,
            &portfolio_value,
            &evm_value,
            &submission_bindings,
            &balance_bindings,
        )?;
        Ok(Self {
            tenant_scope_id: runtime.store_identity().tenant().clone(),
            reader,
            _configuration: configuration,
            _audit: audit,
            runtime,
            portfolio_configuration: portfolio_value,
            portfolio_configuration_head: portfolio_head,
            evm_configuration: evm_value,
            evm_configuration_head: evm_head,
            submission_bindings,
            balance_bindings,
            suspended: Mutex::new(BTreeMap::new()),
        })
    }

    /// Plans and admits one public selector through the mandatory Runtime.
    pub async fn admit_run(&self, request: AdmitRunRequest) -> Result<AdmitRunResponse> {
        if request.entry_point_id.as_str() == PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID {
            let selector: PortfolioSnapshotSelector =
                serde_json::from_value(request.selector.clone()).map_err(admission_invalid)?;
            let plan = plan_snapshot(
                selector,
                &self.portfolio_configuration,
                &self.balance_bindings,
            )
            .map_err(|_| admission_invalid(()))?;
            let (input, document, source_refs) = plan.into_parts();
            return self
                .admit_planned(
                    request.entry_point_id,
                    request.selector,
                    input,
                    document,
                    self.portfolio_configuration_head.clone(),
                    source_refs,
                )
                .await;
        }
        if request.entry_point_id.as_str() == EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID {
            let selector: EvmSubmissionSelector =
                serde_json::from_value(request.selector.clone()).map_err(admission_invalid)?;
            let plan =
                plan_submission(selector, &self.evm_configuration, &self.submission_bindings)
                    .map_err(|_| admission_invalid(()))?;
            let (input, document, source_refs) = plan.into_parts();
            return self
                .admit_planned(
                    request.entry_point_id,
                    request.selector,
                    input,
                    document,
                    self.evm_configuration_head.clone(),
                    source_refs,
                )
                .await;
        }
        Err(PublicError::BadRequest {
            code: "EntryPointNotFound",
            message: "The entry point is not registered",
        })
    }

    async fn admit_planned<T: MfmValue>(
        &self,
        entry_point_id: StableId,
        selector: serde_json::Value,
        input: T,
        document: ProgramDocument,
        configuration: ResolvedConfigurationHead,
        source_refs: Vec<mfm_ids::ContentRef>,
    ) -> Result<AdmitRunResponse> {
        let catalog = self.runtime.catalog();
        let contract = document.admitted_context_contract_ref().clone();
        let program = catalog
            .program(document)
            .map_err(|_| PublicError::Internal)?;
        let value = catalog
            .qualify(contract, input)
            .map_err(|_| PublicError::Internal)?;
        let run_id = run_id(&entry_point_id, &selector, &self.tenant_scope_id)?;
        let owner = self
            .runtime
            .admission(run_id.clone(), program, value, configuration, source_refs)
            .map_err(|_| PublicError::Internal)?;
        match owner.spawn().await {
            SpawnStep::Active(_) | SpawnStep::Terminal(_) => Ok(AdmitRunResponse {
                run_id,
                disposition: "accepted",
            }),
            SpawnStep::Suspended(suspended) => {
                self.retain_suspended(run_id.clone(), suspended)?;
                Ok(AdmitRunResponse {
                    run_id,
                    disposition: "acknowledgement_unknown",
                })
            }
            SpawnStep::Conflict(_) => Err(PublicError::BadRequest {
                code: "AdmissionConflict",
                message: "The run identity already has a different admission",
            }),
            SpawnStep::Failed(_) => Err(PublicError::Internal),
        }
    }

    /// Advances one Runtime step for a retained run.
    pub async fn drive(&self, run_id: RunId) -> Result<DriveResponse> {
        match self.runtime.resume_run(run_id).await {
            ResumeStep::Active(session) => self.finish_runtime_step(session.drive().await).await,
            ResumeStep::Terminal(terminal) => Ok(DriveResponse {
                run_id: terminal.run_id().clone(),
                head_sequence: terminal.head_sequence(),
                status: status_from_frames(terminal.qualified_run().frames()),
            }),
            ResumeStep::Parked(parked) => Ok(DriveResponse {
                run_id: parked.run_id().clone(),
                head_sequence: parked.head_sequence(),
                status: RunStatus::WaitingPreparation,
            }),
            ResumeStep::Failed(_) => Err(PublicError::RunNotFound),
        }
    }

    /// Resolves one retained Runtime acknowledgement owner by run identity.
    pub async fn resolve_suspended_run(&self, run_id: RunId) -> Result<DriveResponse> {
        let suspended = self
            .suspended
            .lock()
            .map_err(|_| PublicError::Internal)?
            .remove(&run_id)
            .ok_or(PublicError::Internal)?;
        self.finish_runtime_step(suspended.resolve().await).await
    }

    async fn finish_runtime_step(&self, step: RuntimeStep) -> Result<DriveResponse> {
        match step {
            RuntimeStep::Advanced(session) => Ok(DriveResponse {
                run_id: session.run_id().clone(),
                head_sequence: session.head_sequence(),
                status: RunStatus::Ready,
            }),
            RuntimeStep::Terminal(terminal) => Ok(DriveResponse {
                run_id: terminal.run_id().clone(),
                head_sequence: terminal.head_sequence(),
                status: status_from_frames(terminal.qualified_run().frames()),
            }),
            RuntimeStep::PreparationRejected { session, .. } => Ok(DriveResponse {
                run_id: session.run_id().clone(),
                head_sequence: session.head_sequence(),
                status: RunStatus::Ready,
            }),
            RuntimeStep::Unresolved { session, .. } | RuntimeStep::Parked { session, .. } => {
                Ok(DriveResponse {
                    run_id: session.run_id().clone(),
                    head_sequence: session.head_sequence(),
                    status: RunStatus::WaitingPreparation,
                })
            }
            RuntimeStep::Suspended(suspended) => {
                let run_id = suspended.run_id().clone();
                self.retain_suspended(run_id.clone(), suspended)?;
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
            RuntimeStep::ConclusionRejected { .. }
            | RuntimeStep::AdmissionRejected(_)
            | RuntimeStep::Conflict { .. }
            | RuntimeStep::Failed { .. } => Err(PublicError::Internal),
        }
    }

    fn retain_suspended(&self, run_id: RunId, suspended: SuspendedRun) -> Result<()> {
        let mut owners = self.suspended.lock().map_err(|_| PublicError::Internal)?;
        if owners.insert(run_id, Box::new(suspended)).is_some() {
            return Err(PublicError::Internal);
        }
        Ok(())
    }

    /// Reads one run inside this fixed tenant partition without invoking live State code.
    pub async fn read_public_run(&self, run_id: RunId) -> Result<PublicRunView> {
        let run = self.reader.load(&run_id).await.map_err(map_store_error)?;
        let report = qualify_with_retained_program(&run, &self.runtime.catalog())
            .map_err(|_| PublicError::Internal)?;
        Ok(PublicRunView {
            run_id,
            tenant_scope_id: self.tenant_scope_id.clone(),
            head_sequence: run.head_sequence(),
            status: if report.terminal() {
                status_from_frames(run.frames())
            } else if matches!(
                run.frames().last().map(RunFrame::record),
                Some(RunRecord::StatePrepared(_))
            ) {
                RunStatus::WaitingPreparation
            } else {
                RunStatus::Ready
            },
        })
    }

    /// Replays a retained prefix with zero live callbacks.
    pub async fn replay_run(&self, run_id: RunId) -> Result<ReplayResponse> {
        let run = self.reader.load(&run_id).await.map_err(map_store_error)?;
        qualify_with_retained_program(&run, &self.runtime.catalog())
            .map(Into::into)
            .map_err(|_| PublicError::Internal)
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
        let mut entries = BTreeMap::new();
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
        let bytes = PortableRun::from_run(&run)
            .encode()
            .map_err(|_| PublicError::Internal)?;
        Ok(ExportedRun { bytes })
    }
}

/// Builds the exact catalog required by the two current domain entry points.
pub fn application_catalog() -> Result<ProgramCatalog> {
    let mut builder = ProgramCatalog::builder();
    register_catalog_values(&mut builder)?;
    let root = nominal_contract_ref::<EvmSubmissionRequest>().map_err(|_| PublicError::Internal)?;
    let placeholder = ProgramDocument::new(
        StableId::new("mfm.application-catalog@1").map_err(|_| PublicError::Internal)?,
        root.clone(),
        root,
        Vec::new(),
    )
    .map_err(|_| PublicError::Internal)?;
    builder
        .finish(placeholder)
        .map(|(catalog, _)| catalog)
        .map_err(|_| PublicError::Internal)
}

fn register_catalog_values(builder: &mut ProgramCatalogBuilder) -> Result<()> {
    macro_rules! value {
        ($ty:ty) => {
            builder
                .register_value::<$ty>()
                .map_err(|_| PublicError::Internal)?
        };
    }
    value!(EvmSubmissionRequest);
    value!(EvmSubmissionProgress);
    value!(EvmSubmissionOutput);
    value!(EvmSubmissionFailure);
    value!(EvmBalanceContext<PortfolioContinuation>);
    value!(EvmBalanceAsset<PortfolioContinuation>);
    value!(EvmBalanceFailure);
    value!(EvmBalanceCollectionCompletion<PortfolioContinuation>);
    value!(PortfolioSnapshotInput);
    value!(PortfolioContinuation);
    value!(PortfolioSnapshotOutput);
    value!(PortfolioSnapshotFailure);
    builder
        .register_capability::<EvmCapability<0>>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_capability::<EvmCapability<1>>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_capability::<EvmCapability<2>>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_capability::<EvmCapability<3>>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_capability::<EvmCapability<6>>()
        .map_err(|_| PublicError::Internal)?;
    builder
        .register_capability::<EvmCapability<7>>()
        .map_err(|_| PublicError::Internal)?;
    Ok(())
}

fn validate_runtime_closure(
    runtime: &Runtime,
    portfolio: &PortfolioConfig,
    evm: &EvmConfig,
    submission_bindings: &[EvmSubmissionBindings],
    balance_bindings: &[EvmBalanceBindings],
) -> Result<()> {
    let catalog = runtime.catalog();
    let document = snapshot_closure_document(portfolio, balance_bindings)
        .map_err(|_| PublicError::Internal)?;
    let program = catalog
        .program(document)
        .map_err(|_| PublicError::Internal)?;
    runtime
        .validate_program(&program)
        .map_err(|_| PublicError::Internal)?;
    for document in
        submission_closure_documents(evm, submission_bindings).map_err(|_| PublicError::Internal)?
    {
        let program = catalog
            .program(document)
            .map_err(|_| PublicError::Internal)?;
        runtime
            .validate_program(&program)
            .map_err(|_| PublicError::Internal)?;
    }
    Ok(())
}

fn validate_transport_value(value: &serde_json::Value) -> Result<()> {
    if contains_float(value) {
        return Err(PublicError::BadRequest {
            code: "AdmissionRequestInvalid",
            message: "Admission input is not a bounded secret-free canonical value",
        });
    }
    let encoded = serde_json::to_string(value).map_err(|_| PublicError::Internal)?;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&encoded).map_err(|_| PublicError::BadRequest {
            code: "AdmissionRequestInvalid",
            message: "Admission input is not canonical JSON",
        })?;
    if canonical.as_bytes().len() > MAX_ADMISSION_BYTES {
        return Err(PublicError::BadRequest {
            code: "AdmissionRequestInvalid",
            message: "Admission input exceeds its bounded size",
        });
    }
    Ok(())
}

fn admission_invalid(_: impl Sized) -> PublicError {
    PublicError::BadRequest {
        code: "AdmissionRequestInvalid",
        message: "Admission selector does not match its bounded domain contract",
    }
}

fn run_id(
    entry_point: &StableId,
    selector: &serde_json::Value,
    tenant: &TenantScopeId,
) -> Result<RunId> {
    let identity = serde_json::json!({
        "entry_point_id": entry_point.as_str(),
        "selector": selector,
        "tenant_scope_id": tenant.as_str(),
    });
    let encoded = serde_json::to_string(&identity).map_err(|_| PublicError::Internal)?;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&encoded).map_err(|_| PublicError::Internal)?;
    Ok(RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(canonical.as_bytes()),
    ))
}

fn map_store_error(error: mfm_store::StoreError) -> PublicError {
    match error {
        mfm_store::StoreError::NotFound => PublicError::RunNotFound,
        _ => PublicError::Internal,
    }
}

fn status_from_frames(frames: &[RunFrame]) -> RunStatus {
    match frames.last().map(RunFrame::record) {
        Some(RunRecord::StatePrepared(_)) => RunStatus::WaitingPreparation,
        Some(RunRecord::StateConcluded(conclusion))
            if matches!(
                conclusion.outcome(),
                mfm_journal::single_trust::StateOutcome::Failure(_)
            ) =>
        {
            RunStatus::Failed
        }
        Some(RunRecord::StateConcluded(_)) => RunStatus::Terminal,
        Some(RunRecord::RunAdmitted(_)) | None => RunStatus::Ready,
    }
}

fn contains_float(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(number) => number.is_f64(),
        serde_json::Value::Array(values) => values.iter().any(contains_float),
        serde_json::Value::Object(values) => values.values().any(contains_float),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::String(_) => {
            false
        }
    }
}
