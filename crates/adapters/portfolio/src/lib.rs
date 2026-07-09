#![warn(missing_docs)]
//! Portfolio adapter runners for fact-backed report-only portfolio snapshots.
//!
//! This crate binds certified portfolio state descriptors to typed runners over explicit artifact
//! and Platform fact-index capability contracts. Live chain transports are not used by the report
//! graph after the collectors cutover.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_artifact_capabilities::{fact_response_artifact_requirement, hydrate_fact_response_json};
use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1 as events;
use mfm_fact_capabilities::{
    FactIndexReadEvidence, FactIndexReadProvider, FactIndexReadRequest, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial,
};
use mfm_facts::{
    compile_fact_query_plan, FactAudience, FactCanonicalScalar, FactFieldId, FactOrderingName,
    FactQueryInput, FactQueryOperator, FactQueryPredicate, FactQueryScope, FactVisibilityScope,
    ScopeDecisionEvidence, StoreScopeRef,
};
use mfm_ids::{ContentDigest, DigestAlgorithm};
use mfm_program::{MfmFactType, StateSpec, ValidatedConfig};
use mfm_runtime::{
    load_materialized_input_value, load_materialized_struct_input, load_runner_config,
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, RunnerExecutableIdentityTemplate,
    RunnerOutputBuilder, RunnerRegistrationBuilder,
};
use mfm_state_portfolio::{
    assemble_snapshot, balance_reader_kind, expand_required_holdings,
    observations_from_selected_holdings, portfolio_adapter_kind, portfolio_adapter_version,
    project_network_pins_from_observations, resolve_subjects_from_config,
    resolve_valuations_from_config, select_network_coherent, symbols_by_id_map, HoldingAnchor,
    AssembleSnapshotConfig, AssembleSnapshotInput, AssembleSnapshotState, HoldingCandidate,
    HoldingFactProjection, PortfolioHoldingErrorCode, PortfolioHoldingSelectionError,
    ProjectReportConfig, ProjectReportInput, ProjectReportState, RequiredHoldingKey,
    RequiredHoldingRequirement, ResolveSubjectsConfig, ResolveSubjectsState,
    ResolveValuationsConfig, ResolveValuationsState, SelectHoldingsConfig, SelectHoldingsState,
    SelectedHoldingMaterial, SelectedHoldings,
};
use mfm_states_btc::{
    normalize_btc_address_balance, BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact,
    BtcAddressBalanceSubject,
};
use mfm_states_evm::{
    normalize_evm_address_native_balance, EvmAddressNativeBalanceResponse,
    EvmAddressNativeBalanceSnapshotFact, EvmAddressNativeBalanceSubject,
};
use mfm_portfolio_model::symbol::ObservationAnchor;
use mfm_store::v1 as store;
use mfm_values::MfmValue;

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const ADAPTER_FACTORY: &str = "portfolio_adapter";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.portfolio.runtime.v1";

/// Runtime capabilities used by portfolio adapter runners.
#[derive(Clone)]
pub struct PortfolioRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl PortfolioRunnerCapabilities {
    /// Creates portfolio runner capabilities from artifact and Platform fact-index providers.
    pub fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        fact_index: Arc<dyn FactIndexReadProvider>,
    ) -> Self {
        Self {
            artifacts,
            fact_index,
        }
    }

    fn artifacts(&self) -> Arc<dyn store::RetainedArtifactReadProvider> {
        Arc::clone(&self.artifacts)
    }

    fn fact_index(&self) -> Arc<dyn FactIndexReadProvider> {
        Arc::clone(&self.fact_index)
    }
}

/// Registers typed portfolio runners (report-only; Platform fact-index for SelectHoldings).
pub fn register_portfolio_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: PortfolioRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let artifacts = capabilities.artifacts();
    let fact_index = capabilities.fact_index();
    let mut registrations = RunnerRegistrationBuilder::new(registry, implementation_id);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-portfolio",
        "typed-portfolio",
        env!("CARGO_PKG_VERSION"),
    )?;
    let pure_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(PURE_FACTORY)?);
    let read_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(READ_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    registrations.register_adapter_executable_with_factory(
        portfolio_adapter_kind()?,
        portfolio_adapter_version()?,
        &adapter_factory,
    )?;
    registrations.register_state_descriptor_with_factory::<ResolveSubjectsState>(
        &pure_factory,
        Arc::new(ResolveSubjectsRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<SelectHoldingsState>(
        &read_factory,
        Arc::new(SelectHoldingsRunner {
            artifacts: artifacts.clone(),
            fact_index,
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<ResolveValuationsState>(
        &pure_factory,
        Arc::new(ResolveValuationsRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<AssembleSnapshotState>(
        &pure_factory,
        Arc::new(AssembleSnapshotRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<ProjectReportState>(
        &pure_factory,
        Arc::new(ProjectReportRunner { artifacts }),
    )?;
    Ok(())
}

struct ResolveSubjectsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for ResolveSubjectsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ResolveSubjectsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let output = resolve_subjects_from_config(config.as_ref());
            state_output(ctx, &output)
        })
    }
}

struct SelectHoldingsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl ErasedNodeRunner for SelectHoldingsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<SelectHoldingsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let subjects = load_materialized_input_value::<mfm_state_portfolio::ResolvedSubjects>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let (selected, evidences) = select_holdings(
                config,
                subjects,
                self.artifacts.as_ref(),
                self.fact_index.as_ref(),
            )
            .await?;
            select_holdings_output(ctx, &selected, &evidences)
        })
    }
}

struct ResolveValuationsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for ResolveValuationsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ResolveValuationsConfig>(&ctx, self.artifacts.as_ref())
                    .await?;
            let output = resolve_valuations_from_config(config.as_ref())
                .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
            state_output(ctx, &output)
        })
    }
}

struct AssembleSnapshotRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleSnapshotRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<AssembleSnapshotConfig>(&ctx, self.artifacts.as_ref()).await?;
            let input = load_materialized_struct_input::<AssembleSnapshotInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = assemble_snapshot(config.as_ref(), input, 0)
                .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
            state_output(ctx, &output)
        })
    }
}

struct ProjectReportRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for ProjectReportRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ProjectReportConfig>(&ctx, self.artifacts.as_ref()).await?;
            let input = load_materialized_struct_input::<ProjectReportInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = mfm_state_portfolio::project_report_from_snapshot(
                input.snapshot,
                config.as_ref().report_version(),
            )
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
            state_output(ctx, &output)
        })
    }
}

async fn select_holdings(
    config: ValidatedConfig<SelectHoldingsConfig>,
    subjects: mfm_state_portfolio::ResolvedSubjects,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    fact_index: &dyn FactIndexReadProvider,
) -> mfm_runtime::Result<(SelectedHoldings, Vec<FactIndexReadEvidence>)> {
    let state = SelectHoldingsState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let config = state.config();
    let requirements = expand_required_holdings(config, &subjects).map_err(holding_runtime_error)?;

    let mut candidates_by_holding: BTreeMap<RequiredHoldingKey, Vec<HoldingCandidate>> =
        BTreeMap::new();
    let mut per_holding_claim_ids: BTreeMap<RequiredHoldingKey, Vec<String>> = BTreeMap::new();
    let mut request_response: Vec<(
        RequiredHoldingKey,
        mfm_fact_capabilities::FactIndexReadRequest,
        FactIndexReadResponse,
    )> = Vec::new();

    for requirement in &requirements {
        let request =
            holding_fact_index_request(config, requirement).map_err(holding_runtime_error)?;
        let response = fact_index
            .read_fact_index(&request)
            .await
            .map_err(fact_index_runtime_error)?;
        let (candidates, claim_ids) =
            candidates_for_requirement(requirement, &response, artifacts).await?;
        candidates_by_holding.insert(requirement.key.clone(), candidates);
        per_holding_claim_ids.insert(requirement.key.clone(), claim_ids);
        request_response.push((requirement.key.clone(), request, response));
    }

    // Ensure every required holding key is present (even if empty) for pure select.
    for requirement in &requirements {
        candidates_by_holding
            .entry(requirement.key.clone())
            .or_default();
    }

    let selected = select_network_coherent(&candidates_by_holding).map_err(holding_runtime_error)?;
    let selected_claim_ids: BTreeSet<String> = selected
        .iter()
        .map(|item| item.fact_claim_id.clone())
        .collect();

    let symbols = symbols_by_id_map(&config.portfolio().symbol_configs).map_err(holding_runtime_error)?;
    let observations =
        observations_from_selected_holdings(&selected, &symbols).map_err(holding_runtime_error)?;
    // Guard: residual pin projection consistency (also enforced at assemble).
    let _ = project_network_pins_from_observations(&observations).map_err(holding_runtime_error)?;

    let mut evidences = Vec::with_capacity(request_response.len());
    for (key, request, response) in request_response {
        let row_claim_ids = per_holding_claim_ids.get(&key).cloned().unwrap_or_default();
        let selection = state
            .selection_evidence_for_claims(&row_claim_ids, &selected_claim_ids)
            .map_err(holding_runtime_error)?;
        let evidence = response
            .into_evidence(&request, selection)
            .map_err(fact_index_runtime_error)?;
        evidences.push(evidence);
    }

    Ok((SelectedHoldings { observations }, evidences))
}

async fn candidates_for_requirement(
    requirement: &RequiredHoldingRequirement,
    response: &FactIndexReadResponse,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<(Vec<HoldingCandidate>, Vec<String>)> {
    let mut candidates = Vec::new();
    let mut claim_ids = Vec::new();
    for row in response.rows() {
        let fact_ref = row.fact_ref();
        let claim_id = fact_claim_id_string(fact_ref.fact_claim_id());
        claim_ids.push(claim_id.clone());
        let store_commit_order = store_commit_order_from_row(row).ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "missing_fact: holding fact row missing store_commit_order".to_owned(),
            )
        })?;
        let requirement_artifact = fact_response_artifact_requirement(fact_ref);
        let artifact = artifacts
            .read_retained_artifact(&requirement_artifact)
            .await
            .map_err(runtime_artifact_read_error)?;
        match requirement.projection {
            HoldingFactProjection::BitcoinAddressBalance => {
                let hydrated: BtcAddressBalanceResponse =
                    hydrate_fact_response_json(fact_ref, artifact.bytes()).map_err(|error| {
                        mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                    })?;
                match btc_holding_candidate(requirement, &hydrated, store_commit_order, claim_id) {
                    Ok(candidate) => candidates.push(candidate),
                    // Coverage/status filter-empty collapses to missing_fact at select time.
                    Err(error)
                        if error.code == PortfolioHoldingErrorCode::MissingFact
                            || error.code == PortfolioHoldingErrorCode::UnsupportedRequirement => {}
                    Err(error) => return Err(holding_runtime_error(error)),
                }
            }
            HoldingFactProjection::EvmNativeBalance => {
                let hydrated: EvmAddressNativeBalanceResponse =
                    hydrate_fact_response_json(fact_ref, artifact.bytes()).map_err(|error| {
                        mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                    })?;
                match evm_holding_candidate(requirement, &hydrated, store_commit_order, claim_id) {
                    Ok(candidate) => candidates.push(candidate),
                    Err(error)
                        if error.code == PortfolioHoldingErrorCode::MissingFact
                            || error.code == PortfolioHoldingErrorCode::UnsupportedRequirement => {}
                    Err(error) => return Err(holding_runtime_error(error)),
                }
            }
        }
    }
    Ok((candidates, claim_ids))
}

fn holding_fact_index_request(
    config: &SelectHoldingsConfig,
    requirement: &RequiredHoldingRequirement,
) -> Result<FactIndexReadRequest, PortfolioHoldingSelectionError> {
    let store_scope = StoreScopeRef::new(config.store_scope()).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    let (descriptor, predicates, return_fields, ordering) = match requirement.projection {
        HoldingFactProjection::BitcoinAddressBalance => {
            let descriptor = BtcAddressBalanceSnapshotFact::descriptor().map_err(|error| {
                PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    error.to_string(),
                    Some(requirement.key.as_key_str()),
                    Some(requirement.key.network_id.clone()),
                )
            })?;
            let bitcoin_network = requirement.network.bitcoin_network().ok_or_else(|| {
                PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    "bitcoin network tag missing",
                    Some(requirement.key.as_key_str()),
                    Some(requirement.key.network_id.clone()),
                )
            })?;
            let source_identity = requirement
                .network
                .source_identity()
                .map(|id| id.to_string())
                .ok_or_else(|| {
                    PortfolioHoldingSelectionError::new(
                        PortfolioHoldingErrorCode::UnsupportedRequirement,
                        "bitcoin source identity missing",
                        Some(requirement.key.as_key_str()),
                        Some(requirement.key.network_id.clone()),
                    )
                })?;
            (
                descriptor,
                vec![
                    equal_predicate("subject.network", requirement.key.network_id.as_str())?,
                    equal_predicate("subject.bitcoin_network", bitcoin_network)?,
                    equal_predicate("subject.semantic_source_identity", &source_identity)?,
                    equal_predicate("subject.address", &requirement.address)?,
                ],
                vec![
                    field_id("result.anchor_height")?,
                    field_id("result.anchor_hash")?,
                    field_id("result.balance_sats")?,
                    field_id("result.coverage")?,
                    field_id("result.source_status")?,
                    field_id("metadata.store_commit_order")?,
                ],
                FactOrderingName::new("result.anchor_height.desc").map_err(|error| {
                    PortfolioHoldingSelectionError::new(
                        PortfolioHoldingErrorCode::UnsupportedRequirement,
                        error.to_string(),
                        Some(requirement.key.as_key_str()),
                        Some(requirement.key.network_id.clone()),
                    )
                })?,
            )
        }
        HoldingFactProjection::EvmNativeBalance => {
            let descriptor =
                EvmAddressNativeBalanceSnapshotFact::descriptor().map_err(|error| {
                    PortfolioHoldingSelectionError::new(
                        PortfolioHoldingErrorCode::UnsupportedRequirement,
                        error.to_string(),
                        Some(requirement.key.as_key_str()),
                        Some(requirement.key.network_id.clone()),
                    )
                })?;
            let chain_id = requirement.network.chain_id_u64().ok_or_else(|| {
                PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    "evm chain_id missing",
                    Some(requirement.key.as_key_str()),
                    Some(requirement.key.network_id.clone()),
                )
            })?;
            (
                descriptor,
                vec![
                    equal_predicate("subject.network", requirement.key.network_id.as_str())?,
                    equal_predicate_u64("subject.chain_id", chain_id)?,
                    equal_predicate("subject.account", &requirement.address)?,
                ],
                vec![
                    field_id("result.block_number")?,
                    field_id("result.block_hash")?,
                    field_id("result.raw_wei")?,
                    field_id("result.decimals")?,
                    field_id("result.coverage")?,
                    field_id("result.source_status")?,
                    field_id("metadata.store_commit_order")?,
                ],
                FactOrderingName::new("result.block_number.desc").map_err(|error| {
                    PortfolioHoldingSelectionError::new(
                        PortfolioHoldingErrorCode::UnsupportedRequirement,
                        error.to_string(),
                        Some(requirement.key.as_key_str()),
                        Some(requirement.key.network_id.clone()),
                    )
                })?,
            )
        }
    };

    let input = FactQueryInput::new(
        store_scope,
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(selection_scope_decision_hash()),
        predicates,
        return_fields,
        ordering,
        None,
    )
    .map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    let plan = compile_fact_query_plan(&descriptor, input).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    FactIndexReadRequest::new(plan).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })
}

fn btc_holding_candidate(
    requirement: &RequiredHoldingRequirement,
    response: &BtcAddressBalanceResponse,
    store_commit_order: u64,
    fact_claim_id: impl Into<String>,
) -> Result<HoldingCandidate, PortfolioHoldingSelectionError> {
    let subject = BtcAddressBalanceSubject::new(
        requirement.key.network_id.clone(),
        requirement
            .network
            .bitcoin_network()
            .unwrap_or_default()
            .to_owned(),
        requirement
            .network
            .source_identity()
            .map(|id| id.to_string())
            .unwrap_or_default(),
        requirement.address.clone(),
    )
    .map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    let normalized = normalize_btc_address_balance(&subject, response).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    let decimals = requirement.symbol.decimals.unwrap_or(8);
    let anchor = HoldingAnchor::new(normalized.anchor_height, normalized.anchor_hash.clone())?;
    Ok(HoldingCandidate {
        network_id: requirement.key.network_id.clone(),
        anchor,
        store_commit_order,
        fact_claim_id: fact_claim_id.into(),
        response_material: SelectedHoldingMaterial {
            wallet_id: requirement.key.wallet_id.clone(),
            symbol_id: requirement.key.symbol_id.clone(),
            network_id: requirement.key.network_id.clone(),
            balance_reader_kind: balance_reader_kind(&requirement.symbol.balance_reader).to_owned(),
            raw_dec: normalized.balance_sats.to_string(),
            decimals,
            observation_anchor: ObservationAnchor::Bitcoin {
                height: normalized.anchor_height,
                block_hash: normalized.anchor_hash,
            },
            coverage: normalized.coverage.as_str().to_owned(),
            source_status: normalized.source_status.as_str().to_owned(),
        },
    })
}

fn evm_holding_candidate(
    requirement: &RequiredHoldingRequirement,
    response: &EvmAddressNativeBalanceResponse,
    store_commit_order: u64,
    fact_claim_id: impl Into<String>,
) -> Result<HoldingCandidate, PortfolioHoldingSelectionError> {
    let chain_id = requirement.network.chain_id_u64().ok_or_else(|| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            "evm chain_id missing",
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    let subject = EvmAddressNativeBalanceSubject::new(
        requirement.key.network_id.clone(),
        chain_id,
        requirement.address.clone(),
    )
    .map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    let normalized = normalize_evm_address_native_balance(&subject, response).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    let decimals = requirement.symbol.decimals.unwrap_or(normalized.decimals);
    let anchor = HoldingAnchor::new(normalized.block_number, normalized.block_hash.clone())?;
    Ok(HoldingCandidate {
        network_id: requirement.key.network_id.clone(),
        anchor,
        store_commit_order,
        fact_claim_id: fact_claim_id.into(),
        response_material: SelectedHoldingMaterial {
            wallet_id: requirement.key.wallet_id.clone(),
            symbol_id: requirement.key.symbol_id.clone(),
            network_id: requirement.key.network_id.clone(),
            balance_reader_kind: balance_reader_kind(&requirement.symbol.balance_reader).to_owned(),
            raw_dec: normalized.raw_wei,
            decimals,
            observation_anchor: ObservationAnchor::Evm {
                chain_id: normalized.chain_id,
                block_number: normalized.block_number,
                block_hash: normalized.block_hash,
            },
            coverage: normalized.coverage.as_str().to_owned(),
            source_status: normalized.source_status.as_str().to_owned(),
        },
    })
}

fn selection_scope_decision_hash() -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.portfolio.holding.select.scope.v1"),
    )
}

fn field_id(id: &str) -> Result<FactFieldId, PortfolioHoldingSelectionError> {
    FactFieldId::new(id).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            error.to_string(),
            None,
            None,
        )
    })
}

fn equal_predicate(
    id: &str,
    value: &str,
) -> Result<FactQueryPredicate, PortfolioHoldingSelectionError> {
    Ok(FactQueryPredicate::new(
        field_id(id)?,
        FactQueryOperator::Equal,
        FactCanonicalScalar::string(value),
    ))
}

fn equal_predicate_u64(
    id: &str,
    value: u64,
) -> Result<FactQueryPredicate, PortfolioHoldingSelectionError> {
    Ok(FactQueryPredicate::new(
        field_id(id)?,
        FactQueryOperator::Equal,
        FactCanonicalScalar::UnsignedInteger(value),
    ))
}

fn store_commit_order_from_row(row: &mfm_fact_capabilities::FactIndexReadRow) -> Option<u64> {
    for field in row.returned_fields() {
        if field.field_id().as_str() == "metadata.store_commit_order" {
            if let FactCanonicalScalar::UnsignedInteger(value) = field.value() {
                return Some(*value);
            }
        }
    }
    None
}

fn fact_claim_id_string(claim_id: &mfm_facts::FactClaimId) -> String {
    format!(
        "{}:{}:{}",
        claim_id.source_run_id(),
        claim_id.source_seq(),
        claim_id.source_ordinal()
    )
}

fn select_holdings_output(
    ctx: ErasedRunCtx<'_>,
    value: &SelectedHoldings,
    evidences: &[FactIndexReadEvidence],
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.state_output(value)?;
    for evidence in evidences {
        let trust_root = fact_query_trust_root(evidence.trust_root())?;
        output.record_fact_query_evidence(evidence.query_evidence().clone(), &trust_root)?;
    }
    Ok(output.finish())
}

fn state_output<T>(ctx: ErasedRunCtx<'_>, value: &T) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmValue,
{
    ErasedRunnerOutput::state_output(&ctx, value)
}

fn fact_query_trust_root(
    material: &FactQueryReceiptTrustRootMaterial,
) -> mfm_runtime::Result<store::FactQueryReceiptTrustRoot> {
    store::FactQueryReceiptTrustRoot::from_material(material)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

fn holding_runtime_error(error: PortfolioHoldingSelectionError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
        "{code}: {message}",
        code = error.code.as_str(),
        message = error.message
    ))
}

fn fact_index_runtime_error(
    error: mfm_fact_capabilities::FactIndexReadError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_artifact_read_error(error: store::StoreError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

#[cfg(test)]
mod tests;
