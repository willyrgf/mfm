use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use mfm_collectors_rpc_control::{EvmIoClient, PrepareSourcesResponse, DEFAULT_CONTROL_SCOPE};
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::{ContextKey, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_state_common::ctx::{read_typed, write_json};
use mfm_state_common::errors::{
    state_error_with_state, state_from_io, state_unknown, state_unknown_msg,
};
use mfm_state_common::local_io_helpers::emit_report_event;
use mfm_state_common::output::write_output_artifact;
use mfm_state_common::states::meta;
use mfm_state_symbol::model::{Observation, QuoteCode, SymbolRole};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};
use serde::Deserialize;
use serde_json::Value;

use crate::model::{
    ExecutionAnchor as SnapshotExecutionAnchor, NetworkPin, PortfolioConfig, PortfolioQuoteTotal,
    PortfolioReport, PortfolioSnapshot, PortfolioSnapshotError, WalletReport, WalletSnapshot,
};
use crate::semantic::{
    CompiledObservationBatch, ExecutionAnchor, NetworkFamily, ObservationRuntimeInput,
    PinnedNetworkView, ResolvedSubject, ResolvedUnitPrice, SemanticCatalog, SourcePreparationTask,
    SubjectKind, SubjectResolutionTask, SubjectRuntimeInput, ValuationRuntimeInput, ValuationTask,
    ViewPinTask, ViewRuntimeInput,
};
use mfm_state_wallet::model::WalletSubjectKind;

/// Fixed semantic runtime state that prepares execution sources for compiled tasks.
#[derive(Clone)]
pub struct PrepareExecutionSourcesState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Planner-owned source preparation tasks.
    pub tasks: Vec<SourcePreparationTask>,
    /// Context key that receives the prepared source payloads keyed by network view id.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for PrepareExecutionSourcesState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut tasks = self.tasks.clone();
        tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));

        let mut prepared = BTreeMap::new();
        for task in tasks {
            if prepared.contains_key(task.network_view_id.as_str()) {
                return Err(state_unknown_msg(
                    "duplicate_prepared_source_view",
                    format!(
                        "source preparation emitted duplicate network_view_id `{}`",
                        task.network_view_id
                    ),
                ));
            }

            let value = match task.family {
                NetworkFamily::Evm => {
                    let payload: EvmPreparedSourcesPayload =
                        decode_object_map("source_preparation_task", &task.task_id, &task.payload)?;
                    let control_scope = if payload.control_scope.trim().is_empty() {
                        DEFAULT_CONTROL_SCOPE.to_string()
                    } else {
                        payload.control_scope.clone()
                    };
                    let mut client = EvmIoClient::new(self.state_id.clone(), io);
                    let response = client
                        .prepare_sources_in_scope(control_scope, payload.network_id.clone())
                        .await
                        .map_err(state_from_io)?;
                    if !response.sources.iter().any(|entry| entry.healthy) {
                        return Err(state_error_with_state(
                            self.state_id.clone(),
                            "rpc_control_no_healthy_sources",
                            ErrorCategory::Rpc,
                            false,
                            format!(
                                "no responsive rpc.control sources found for network `{}` and scope `{}`",
                                response.network_id, response.control_scope
                            ),
                        ));
                    }
                    serde_json::to_value(response).map_err(|_| {
                        state_unknown(
                            "prepared_sources_serialize_failed",
                            "failed to serialize prepared source payload",
                        )
                    })?
                }
                NetworkFamily::Bitcoin => {
                    let payload: EvmPreparedSourcesPayload =
                        decode_object_map("source_preparation_task", &task.task_id, &task.payload)?;
                    let control_scope = if payload.control_scope.trim().is_empty() {
                        DEFAULT_CONTROL_SCOPE.to_string()
                    } else {
                        payload.control_scope.clone()
                    };
                    let response = prepare_bitcoin_sources(
                        &self.state_id,
                        io,
                        control_scope,
                        payload.network_id.clone(),
                    )
                    .await?;
                    if !response.sources.iter().any(|entry| entry.healthy) {
                        return Err(state_error_with_state(
                            self.state_id.clone(),
                            "rpc_control_no_healthy_sources",
                            ErrorCategory::Rpc,
                            false,
                            format!(
                                "no responsive rpc.control sources found for bitcoin network `{}` and scope `{}`",
                                response.network_id, response.control_scope
                            ),
                        ));
                    }
                    serde_json::to_value(response).map_err(|_| {
                        state_unknown(
                            "prepared_sources_serialize_failed",
                            "failed to serialize prepared source payload",
                        )
                    })?
                }
            };
            prepared.insert(task.network_view_id, value);
        }

        write_json(
            ctx,
            self.output_key.clone(),
            serde_json::to_value(prepared).map_err(|_| {
                state_unknown(
                    "prepared_sources_serialize_failed",
                    "failed to serialize prepared source map",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Fixed semantic runtime state that resolves compiled subjects.
#[derive(Clone)]
pub struct ResolveSubjectsState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Planner-owned subject resolution tasks.
    pub tasks: Vec<SubjectResolutionTask>,
    /// Runtime adapter catalog keyed by planned adapter id.
    pub catalog: Arc<SemanticCatalog>,
    /// Context key that contains prepared source payloads.
    pub prepared_sources_key: ContextKey,
    /// Context key that receives resolved subjects keyed by semantic subject id.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for ResolveSubjectsState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let prepared_sources: BTreeMap<String, Value> = read_typed(
            ctx,
            &self.prepared_sources_key,
            "missing_prepared_sources",
            "missing prepared sources in context",
            "prepared_sources_decode_failed",
            "failed to decode prepared sources",
        )?;

        let mut tasks = self.tasks.clone();
        tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));

        let mut resolved = BTreeMap::new();
        for task in tasks {
            let runtime = self.catalog.subject_runtime(&task.adapter).map_err(|err| {
                state_unknown_msg(
                    "subject_runtime_adapter_not_found",
                    format!("failed to resolve subject runtime adapter: {err}"),
                )
            })?;
            let subject = runtime
                .resolve_subject(
                    &self.state_id,
                    _io,
                    &task,
                    SubjectRuntimeInput {
                        prepared_sources: &prepared_sources,
                    },
                )
                .await?;
            if subject.subject_id != task.subject_id {
                return Err(state_unknown_msg(
                    "subject_runtime_identity_mismatch",
                    format!(
                        "subject runtime adapter `{}` returned subject_id `{}` for task `{}`",
                        task.adapter, subject.subject_id, task.subject_id
                    ),
                ));
            }
            if subject.kind != task.kind {
                return Err(state_unknown_msg(
                    "subject_runtime_kind_mismatch",
                    format!(
                        "subject runtime adapter `{}` returned kind `{:?}` for task `{}`",
                        task.adapter, subject.kind, task.subject_id
                    ),
                ));
            }
            if resolved
                .insert(subject.subject_id.clone(), subject)
                .is_some()
            {
                return Err(state_unknown_msg(
                    "duplicate_resolved_subject",
                    format!("duplicate resolved subject `{}`", task.subject_id),
                ));
            }
        }

        write_json(
            ctx,
            self.output_key.clone(),
            serde_json::to_value(resolved).map_err(|_| {
                state_unknown(
                    "resolved_subjects_serialize_failed",
                    "failed to serialize resolved subjects",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Fixed semantic runtime state that pins compiled execution views.
#[derive(Clone)]
pub struct PinExecutionViewsState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Planner-owned view pin tasks.
    pub tasks: Vec<ViewPinTask>,
    /// Runtime adapter catalog keyed by planned adapter id.
    pub catalog: Arc<SemanticCatalog>,
    /// Context key that contains prepared source payloads.
    pub prepared_sources_key: ContextKey,
    /// Context key that receives pinned execution views keyed by semantic network view id.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for PinExecutionViewsState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let prepared_sources: BTreeMap<String, Value> = read_typed(
            ctx,
            &self.prepared_sources_key,
            "missing_prepared_sources",
            "missing prepared sources in context",
            "prepared_sources_decode_failed",
            "failed to decode prepared sources",
        )?;

        let mut tasks = self.tasks.clone();
        tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));

        let mut pinned = BTreeMap::new();
        for task in tasks {
            let runtime = self.catalog.view_runtime(&task.adapter).map_err(|err| {
                state_unknown_msg(
                    "view_runtime_adapter_not_found",
                    format!("failed to resolve view runtime adapter: {err}"),
                )
            })?;
            let view = runtime
                .pin_view(
                    &self.state_id,
                    _io,
                    &task,
                    ViewRuntimeInput {
                        prepared_sources: &prepared_sources,
                    },
                )
                .await?;
            if view.network_view_id != task.network_view_id {
                return Err(state_unknown_msg(
                    "view_runtime_identity_mismatch",
                    format!(
                        "view runtime adapter `{}` returned network_view_id `{}` for task `{}`",
                        task.adapter, view.network_view_id, task.network_view_id
                    ),
                ));
            }
            if view.family != task.family {
                return Err(state_unknown_msg(
                    "view_runtime_family_mismatch",
                    format!(
                        "view runtime adapter `{}` returned family `{:?}` for task `{}`",
                        task.adapter, view.family, task.network_view_id
                    ),
                ));
            }
            if pinned.insert(view.network_view_id.clone(), view).is_some() {
                return Err(state_unknown_msg(
                    "duplicate_pinned_view",
                    format!("duplicate pinned view `{}`", task.network_view_id),
                ));
            }
        }

        write_json(
            ctx,
            self.output_key.clone(),
            serde_json::to_value(pinned).map_err(|_| {
                state_unknown(
                    "pinned_views_serialize_failed",
                    "failed to serialize pinned views",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Fixed semantic runtime state that resolves compiled valuation inputs.
#[derive(Clone)]
pub struct ResolveValuationInputsState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Planner-owned valuation tasks.
    pub tasks: Vec<ValuationTask>,
    /// Runtime adapter catalog keyed by planned adapter id.
    pub catalog: Arc<SemanticCatalog>,
    /// Context key that contains pinned execution views.
    pub pinned_views_key: ContextKey,
    /// Context key that receives resolved unit prices keyed by valuation id.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for ResolveValuationInputsState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let pinned_views: BTreeMap<String, PinnedNetworkView> = read_typed(
            ctx,
            &self.pinned_views_key,
            "missing_pinned_views",
            "missing pinned views in context",
            "pinned_views_decode_failed",
            "failed to decode pinned views",
        )?;

        let mut tasks = self.tasks.clone();
        tasks.sort_by(|left, right| left.valuation_id.cmp(&right.valuation_id));

        let mut resolved = BTreeMap::new();
        for task in tasks {
            let runtime = self
                .catalog
                .valuation_runtime(&task.adapter)
                .map_err(|err| {
                    state_unknown_msg(
                        "valuation_runtime_adapter_not_found",
                        format!("failed to resolve valuation runtime adapter: {err}"),
                    )
                })?;
            let value = runtime
                .resolve(
                    &self.state_id,
                    _io,
                    &task,
                    ValuationRuntimeInput {
                        pinned_views: &pinned_views,
                    },
                )
                .await?;
            if value.valuation_id != task.valuation_id {
                return Err(state_unknown_msg(
                    "valuation_runtime_identity_mismatch",
                    format!(
                        "valuation runtime adapter `{}` returned valuation_id `{}` for task `{}`",
                        task.adapter, value.valuation_id, task.valuation_id
                    ),
                ));
            }
            if value.instrument_id != task.instrument_id {
                return Err(state_unknown_msg(
                    "valuation_runtime_instrument_mismatch",
                    format!(
                        "valuation runtime adapter `{}` returned instrument_id `{}` for task `{}`",
                        task.adapter, value.instrument_id, task.valuation_id
                    ),
                ));
            }
            if value.quote != task.quote {
                return Err(state_unknown_msg(
                    "valuation_runtime_quote_mismatch",
                    format!(
                        "valuation runtime adapter `{}` returned quote `{}` for task `{}`",
                        task.adapter, value.quote, task.valuation_id
                    ),
                ));
            }
            if resolved.insert(value.valuation_id.clone(), value).is_some() {
                return Err(state_unknown_msg(
                    "duplicate_resolved_valuation",
                    format!("duplicate resolved valuation `{}`", task.valuation_id),
                ));
            }
        }

        write_json(
            ctx,
            self.output_key.clone(),
            serde_json::to_value(resolved).map_err(|_| {
                state_unknown(
                    "resolved_valuations_serialize_failed",
                    "failed to serialize resolved valuations",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Fixed semantic runtime state that executes one compiled observation batch.
#[derive(Clone)]
pub struct ObserveCompiledBatchState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Planner-owned compiled batch to execute.
    pub batch: CompiledObservationBatch,
    /// Runtime adapter catalog keyed by planned adapter id.
    pub catalog: Arc<SemanticCatalog>,
    /// Context key that contains resolved subjects.
    pub resolved_subjects_key: ContextKey,
    /// Context key that contains pinned execution views.
    pub pinned_views_key: ContextKey,
    /// Context key that contains resolved valuations.
    pub resolved_valuations_key: ContextKey,
    /// Context key that receives the canonical observations emitted by the batch.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for ObserveCompiledBatchState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let resolved_subjects: BTreeMap<String, ResolvedSubject> = read_typed(
            ctx,
            &self.resolved_subjects_key,
            "missing_resolved_subjects",
            "missing resolved subjects in context",
            "resolved_subjects_decode_failed",
            "failed to decode resolved subjects",
        )?;
        let pinned_views: BTreeMap<String, PinnedNetworkView> = read_typed(
            ctx,
            &self.pinned_views_key,
            "missing_pinned_views",
            "missing pinned views in context",
            "pinned_views_decode_failed",
            "failed to decode pinned views",
        )?;
        let resolved_valuations: BTreeMap<String, ResolvedUnitPrice> = read_typed(
            ctx,
            &self.resolved_valuations_key,
            "missing_resolved_valuations",
            "missing resolved valuations in context",
            "resolved_valuations_decode_failed",
            "failed to decode resolved valuations",
        )?;

        let runtime = self
            .catalog
            .observation_runtime(&self.batch.adapter)
            .map_err(|err| {
                state_unknown_msg(
                    "observation_runtime_adapter_not_found",
                    format!("failed to resolve observation runtime adapter: {err}"),
                )
            })?;

        let mut observations = Vec::with_capacity(self.batch.bindings.len());
        for binding in &self.batch.bindings {
            let mut observation = runtime
                .observe(
                    &self.state_id,
                    _io,
                    binding,
                    ObservationRuntimeInput {
                        resolved_subjects: &resolved_subjects,
                        pinned_views: &pinned_views,
                        resolved_valuations: &resolved_valuations,
                    },
                )
                .await?;
            observation.normalize();
            observations.push(observation);
        }

        observations.sort_by(|left, right| {
            (left.wallet_id.as_str(), left.symbol_id.as_str())
                .cmp(&(right.wallet_id.as_str(), right.symbol_id.as_str()))
        });

        write_json(
            ctx,
            self.output_key.clone(),
            serde_json::to_value(observations).map_err(|_| {
                state_unknown(
                    "observations_serialize_failed",
                    "failed to serialize observations",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Fixed semantic runtime state that merges multiple observation producers.
#[derive(Clone, Debug)]
pub struct MergeObservationsState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Context keys that contain `Vec<Observation>` payloads.
    pub input_keys: Vec<ContextKey>,
    /// Context key that receives the merged observations.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for MergeObservationsState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut observations = Vec::new();
        for input_key in &self.input_keys {
            let mut next: Vec<Observation> = read_typed(
                ctx,
                input_key,
                "missing_observations",
                "missing observations in context",
                "observations_decode_failed",
                "failed to decode observations",
            )?;
            observations.append(&mut next);
        }

        observations.sort_by(|left, right| {
            (left.wallet_id.as_str(), left.symbol_id.as_str())
                .cmp(&(right.wallet_id.as_str(), right.symbol_id.as_str()))
        });
        let value = serde_json::to_value(&observations).map_err(|_| {
            state_unknown(
                "observations_serialize_failed",
                "failed to serialize observations",
            )
        })?;
        write_json(ctx, self.output_key.clone(), value)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Fixed semantic runtime state that assembles and persists the canonical snapshot artifact.
#[derive(Clone, Debug)]
pub struct AssembleSnapshotState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Canonical portfolio config used for snapshot/report projection.
    pub portfolio: PortfolioConfig,
    /// Context key that contains resolved subjects.
    pub resolved_subjects_key: ContextKey,
    /// Context key that contains pinned execution views.
    pub pinned_views_key: ContextKey,
    /// Context key that contains merged observations.
    pub observations_key: ContextKey,
    /// Fact key used for the output artifact.
    pub fact_key: FactKey,
    /// Context key that receives the snapshot artifact id.
    pub artifact_id_output_key: ContextKey,
    /// Context key that receives the full snapshot JSON.
    pub snapshot_output_key: ContextKey,
}

#[async_trait]
impl State for AssembleSnapshotState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let resolved_subjects: BTreeMap<String, ResolvedSubject> = read_typed(
            ctx,
            &self.resolved_subjects_key,
            "missing_resolved_subjects",
            "missing resolved subjects in context",
            "resolved_subjects_decode_failed",
            "failed to decode resolved subjects",
        )?;
        let pinned_views: BTreeMap<String, PinnedNetworkView> = read_typed(
            ctx,
            &self.pinned_views_key,
            "missing_pinned_views",
            "missing pinned views in context",
            "pinned_views_decode_failed",
            "failed to decode pinned views",
        )?;
        let observations: Vec<Observation> = read_typed(
            ctx,
            &self.observations_key,
            "missing_observations",
            "missing observations in context",
            "observations_decode_failed",
            "failed to decode observations",
        )?;
        let observations_by_wallet = group_observations_by_wallet(observations);

        let mut network_pins_by_id = BTreeMap::new();
        for pinned in pinned_views.values() {
            let network_pin = match &pinned.anchor {
                ExecutionAnchor::Evm {
                    chain_id,
                    block_number,
                } => NetworkPin {
                    network_id: pinned.network_id.clone(),
                    anchor: SnapshotExecutionAnchor::Evm {
                        chain_id: *chain_id,
                        block_number: *block_number,
                    },
                },
                ExecutionAnchor::Bitcoin { height, block_hash } => NetworkPin {
                    network_id: pinned.network_id.clone(),
                    anchor: SnapshotExecutionAnchor::Bitcoin {
                        height: *height,
                        block_hash: block_hash.clone(),
                    },
                },
            };
            if network_pins_by_id
                .insert(network_pin.network_id.clone(), network_pin)
                .is_some()
            {
                return Err(state_unknown_msg(
                    "duplicate_snapshot_network_pin",
                    format!(
                        "multiple pinned execution views lowered to network `{}` in the v1 snapshot schema",
                        pinned.network_id
                    ),
                ));
            }
        }

        let mut wallets = Vec::with_capacity(self.portfolio.wallets.len());
        let mut portfolio_wallets = self.portfolio.wallets.clone();
        portfolio_wallets.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
        for wallet in portfolio_wallets {
            let subject = resolved_subjects
                .get(wallet.wallet_id.as_str())
                .ok_or_else(|| {
                    state_unknown_msg(
                        "missing_resolved_subject",
                        format!(
                            "resolved subject `{}` was not available for snapshot assembly",
                            wallet.wallet_id
                        ),
                    )
                })?;
            let (address, network_id, subject_kind) = match subject.kind {
                SubjectKind::EvmAddress => {
                    let resolved = decode_subject_value::<EvmResolvedSubjectValue>(
                        subject,
                        "resolved_subject_value",
                    )?;
                    (
                        resolved.address,
                        resolved.network_id,
                        WalletSubjectKind::EvmAddress,
                    )
                }
                SubjectKind::BitcoinAddress => {
                    let resolved = decode_subject_value::<BitcoinResolvedSubjectValue>(
                        subject,
                        "resolved_subject_value",
                    )?;
                    (
                        resolved.address,
                        resolved.network_id,
                        WalletSubjectKind::BitcoinAddress,
                    )
                }
            };
            wallets.push(WalletSnapshot {
                wallet_id: wallet.wallet_id,
                address,
                subject_kind,
                network_id,
                observations: observations_by_wallet
                    .get(subject.subject_id.as_str())
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        wallets.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));

        let mut symbol_configs = self.portfolio.symbol_configs.clone();
        for symbol in &mut symbol_configs {
            symbol.normalize();
        }
        symbol_configs.sort_by(|left, right| left.symbol_id.cmp(&right.symbol_id));

        let generated_at_ms = io.now_millis().await.map_err(state_from_io)?;
        let mut snapshot = PortfolioSnapshot {
            schema_version: 2,
            portfolio_id: self.portfolio.portfolio_id.clone(),
            generated_at_ms,
            network_pins: network_pins_by_id.into_values().collect(),
            wallets,
            symbol_configs,
            errors: Vec::<PortfolioSnapshotError>::new(),
        };
        snapshot.normalize();

        let snapshot_value = serde_json::to_value(&snapshot).map_err(|_| {
            state_unknown(
                "portfolio_snapshot_serialize_failed",
                "failed to serialize portfolio snapshot",
            )
        })?;
        write_json(
            ctx,
            self.snapshot_output_key.clone(),
            snapshot_value.clone(),
        )?;
        write_output_artifact(
            ctx,
            io,
            rec,
            self.fact_key.clone(),
            snapshot_value,
            self.artifact_id_output_key.clone(),
        )
        .await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Fixed semantic runtime state that projects the canonical report from the snapshot artifact.
#[derive(Clone, Debug)]
pub struct ProjectReportState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Context key that contains the canonical snapshot JSON.
    pub snapshot_key: ContextKey,
    /// Context key that receives the report JSON.
    pub output_key: ContextKey,
    /// Domain event name emitted after the report is written.
    pub event_name: &'static str,
}

#[async_trait]
impl State for ProjectReportState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let snapshot: PortfolioSnapshot = read_typed(
            ctx,
            &self.snapshot_key,
            "missing_snapshot",
            "missing snapshot in context",
            "snapshot_decode_failed",
            "failed to decode portfolio snapshot",
        )?;

        let report_quotes = collect_report_quotes(&snapshot);
        let mut portfolio_totals = initialized_quote_totals(&report_quotes);
        let wallet_summaries = snapshot
            .wallets
            .iter()
            .map(|wallet| {
                let wallet_totals = derive_quote_totals(&report_quotes, &wallet.observations)?;
                merge_quote_totals(&mut portfolio_totals, &wallet_totals);
                Ok(WalletReport {
                    wallet_id: wallet.wallet_id.clone(),
                    network_id: wallet.network_id.clone(),
                    totals_by_quote: quote_totals_to_vec(wallet_totals),
                })
            })
            .collect::<Result<Vec<_>, StateError>>()?;

        let mut report = PortfolioReport {
            schema_version: 2,
            portfolio_id: snapshot.portfolio_id.clone(),
            generated_at_ms: snapshot.generated_at_ms,
            network_pins: snapshot.network_pins.clone(),
            wallet_summaries,
            totals_by_quote: quote_totals_to_vec(portfolio_totals),
            error_count: snapshot.errors.len() as u64,
        };
        report.normalize();

        let report_value = serde_json::to_value(&report).map_err(|_| {
            state_unknown(
                "portfolio_report_serialize_failed",
                "failed to serialize portfolio report",
            )
        })?;
        write_json(ctx, self.output_key.clone(), report_value.clone())?;
        emit_report_event(rec, self.event_name, report_value).await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
struct EvmPreparedSourcesPayload {
    network_id: String,
    #[serde(default)]
    control_scope: String,
}

#[derive(Clone, Debug, Deserialize)]
struct EvmResolvedSubjectValue {
    network_id: String,
    address: String,
}

#[derive(Clone, Debug, Deserialize)]
struct BitcoinResolvedSubjectValue {
    network_id: String,
    address: String,
}

async fn prepare_bitcoin_sources(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    control_scope: String,
    network_id: String,
) -> Result<PrepareSourcesResponse, StateError> {
    let response = call_rpc_control_raw(
        state_id,
        io,
        serde_json::json!({
            "kind": "prepare_sources",
            "family": "bitcoin",
            "control_scope": control_scope,
            "network_id": network_id,
        }),
    )
    .await?;
    serde_json::from_value(response).map_err(|err| {
        state_unknown_msg(
            "prepared_source_decode_failed",
            format!("bitcoin prepared source payload decode failed: {err}"),
        )
    })
}

async fn call_rpc_control_raw(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    request: Value,
) -> Result<Value, StateError> {
    let request_id = artifact_id_for_json(&request).map_err(|err| {
        state_unknown_msg(
            "semantic_runtime_request_not_canonical",
            format!("semantic runtime request was not canonical json: {err}"),
        )
    })?;
    io.call(IoCall {
        namespace: "rpc.control".to_string(),
        request,
        fact_key: Some(FactKey(format!(
            "mfm:rpc.control|state:{}|req:{}",
            state_id.as_str(),
            request_id.0
        ))),
    })
    .await
    .map_err(state_from_io)
    .map(|result| result.response)
}

fn decode_object_map<T: for<'de> Deserialize<'de>>(
    entity: &'static str,
    id: &str,
    map: &BTreeMap<String, Value>,
) -> Result<T, StateError> {
    serde_json::from_value(Value::Object(
        map.iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    ))
    .map_err(|err| {
        state_unknown_msg(
            "semantic_runtime_payload_decode_failed",
            format!("{entity} `{id}` payload decode failed: {err}"),
        )
    })
}

fn decode_subject_value<T: for<'de> Deserialize<'de>>(
    subject: &ResolvedSubject,
    field: &'static str,
) -> Result<T, StateError> {
    serde_json::from_value(subject.value.clone()).map_err(|err| {
        state_unknown_msg(
            "semantic_runtime_subject_decode_failed",
            format!(
                "resolved subject `{}` field `{field}` decode failed: {err}",
                subject.subject_id
            ),
        )
    })
}

fn group_observations_by_wallet(
    mut observations: Vec<Observation>,
) -> HashMap<String, Vec<Observation>> {
    let mut grouped: HashMap<String, Vec<Observation>> = HashMap::new();
    observations.sort_by(|left, right| {
        (left.wallet_id.as_str(), left.symbol_id.as_str())
            .cmp(&(right.wallet_id.as_str(), right.symbol_id.as_str()))
    });
    for observation in observations {
        grouped
            .entry(observation.wallet_id.clone())
            .or_default()
            .push(observation);
    }
    grouped
}

fn collect_report_quotes(snapshot: &PortfolioSnapshot) -> Vec<QuoteCode> {
    let mut quotes = std::collections::BTreeSet::new();
    for symbol in &snapshot.symbol_configs {
        for quote in &symbol.valuation.quotes {
            quotes.insert(quote.quote);
        }
    }
    for wallet in &snapshot.wallets {
        for observation in &wallet.observations {
            for value in &observation.values {
                quotes.insert(value.quote);
            }
        }
    }
    quotes.into_iter().collect()
}

fn derive_quote_totals(
    report_quotes: &[QuoteCode],
    observations: &[Observation],
) -> Result<BTreeMap<QuoteCode, QuoteTotalsAccumulator>, StateError> {
    let mut totals = initialized_quote_totals(report_quotes);
    for observation in observations {
        for value in &observation.values {
            let entry = totals.entry(value.quote).or_default();
            match observation.role {
                SymbolRole::Native | SymbolRole::Asset => {
                    entry.assets_value = entry
                        .assets_value
                        .add(&DecimalValue::parse(&value.value_dec)?);
                }
                SymbolRole::Collateral => {
                    entry.collateral_value = entry
                        .collateral_value
                        .add(&DecimalValue::parse(&value.value_dec)?);
                }
                SymbolRole::Debt => {
                    entry.debt_value = entry
                        .debt_value
                        .add(&DecimalValue::parse(&value.value_dec)?);
                }
                SymbolRole::Staked => {
                    entry.staked_value = entry
                        .staked_value
                        .add(&DecimalValue::parse(&value.value_dec)?);
                }
            }
        }
    }
    Ok(totals)
}

fn initialized_quote_totals(
    report_quotes: &[QuoteCode],
) -> BTreeMap<QuoteCode, QuoteTotalsAccumulator> {
    report_quotes
        .iter()
        .copied()
        .map(|quote| (quote, QuoteTotalsAccumulator::default()))
        .collect()
}

fn merge_quote_totals(
    target: &mut BTreeMap<QuoteCode, QuoteTotalsAccumulator>,
    source: &BTreeMap<QuoteCode, QuoteTotalsAccumulator>,
) {
    for (quote, totals) in source {
        target
            .entry(*quote)
            .and_modify(|acc| acc.merge(totals))
            .or_insert_with(|| totals.clone());
    }
}

fn quote_totals_to_vec(
    totals: BTreeMap<QuoteCode, QuoteTotalsAccumulator>,
) -> Vec<PortfolioQuoteTotal> {
    totals
        .into_iter()
        .map(|(quote, totals)| totals.into_report_total(quote))
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct QuoteTotalsAccumulator {
    assets_value: DecimalValue,
    collateral_value: DecimalValue,
    debt_value: DecimalValue,
    staked_value: DecimalValue,
}

impl QuoteTotalsAccumulator {
    fn merge(&mut self, other: &Self) {
        self.assets_value = self.assets_value.add(&other.assets_value);
        self.collateral_value = self.collateral_value.add(&other.collateral_value);
        self.debt_value = self.debt_value.add(&other.debt_value);
        self.staked_value = self.staked_value.add(&other.staked_value);
    }

    fn into_report_total(self, quote: QuoteCode) -> PortfolioQuoteTotal {
        let positive_value = self
            .assets_value
            .add(&self.collateral_value)
            .add(&self.staked_value);
        let net_value = positive_value.sub(&self.debt_value);
        PortfolioQuoteTotal {
            quote,
            assets_value_dec: self.assets_value.to_canonical_string(),
            collateral_value_dec: self.collateral_value.to_canonical_string(),
            debt_value_dec: self.debt_value.to_canonical_string(),
            staked_value_dec: self.staked_value.to_canonical_string(),
            net_value_dec: net_value.to_canonical_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecimalValue {
    digits: BigInt,
    scale: u32,
}

impl Default for DecimalValue {
    fn default() -> Self {
        Self::zero()
    }
}

impl DecimalValue {
    fn zero() -> Self {
        Self {
            digits: BigInt::ZERO,
            scale: 0,
        }
    }

    fn parse(input: &str) -> Result<Self, StateError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(state_unknown_msg(
                "invalid_decimal_string",
                format!("invalid decimal string `{input}`"),
            ));
        }

        let (negative, digits_part) = match trimmed.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, trimmed),
        };
        let parts: Vec<_> = digits_part.split('.').collect();
        if parts.len() > 2
            || parts
                .iter()
                .any(|part| !part.is_empty() && !part.chars().all(|ch| ch.is_ascii_digit()))
        {
            return Err(state_unknown_msg(
                "invalid_decimal_string",
                format!("invalid decimal string `{input}`"),
            ));
        }

        let whole = parts[0];
        let frac = parts.get(1).copied().unwrap_or("");
        let digits = format!("{whole}{frac}");
        let digits = if digits.is_empty() {
            "0"
        } else {
            digits.as_str()
        };
        let mut parsed: BigInt = digits.parse().map_err(|_| {
            state_unknown_msg(
                "invalid_decimal_string",
                format!("invalid decimal string `{input}`"),
            )
        })?;
        if negative && !parsed.is_zero() {
            parsed = -parsed;
        }
        Ok(Self {
            digits: parsed,
            scale: frac.len() as u32,
        })
    }

    fn add(&self, other: &Self) -> Self {
        let scale = self.scale.max(other.scale);
        Self {
            digits: self.scaled_digits(scale) + other.scaled_digits(scale),
            scale,
        }
    }

    fn sub(&self, other: &Self) -> Self {
        let scale = self.scale.max(other.scale);
        Self {
            digits: self.scaled_digits(scale) - other.scaled_digits(scale),
            scale,
        }
    }

    fn scaled_digits(&self, scale: u32) -> BigInt {
        if self.scale == scale {
            self.digits.clone()
        } else {
            &self.digits * ten_pow(scale - self.scale)
        }
    }

    fn to_canonical_string(&self) -> String {
        self.to_string_with_min_scale(self.scale)
    }

    fn to_string_with_min_scale(&self, min_scale: u32) -> String {
        let negative = self.digits.is_negative();
        let digits = self.digits.abs().to_string();
        let scale = self.scale as usize;
        let mut out = if scale == 0 {
            digits
        } else if digits.len() <= scale {
            format!("0.{}{}", "0".repeat(scale - digits.len()), digits)
        } else {
            let split = digits.len() - scale;
            format!("{}.{}", &digits[..split], &digits[split..])
        };

        if let Some((whole, frac)) = out.split_once('.') {
            let mut frac = frac.to_string();
            while frac.len() > min_scale as usize && frac.ends_with('0') {
                frac.pop();
            }
            if frac.len() < min_scale as usize {
                frac.push_str(&"0".repeat(min_scale as usize - frac.len()));
            }
            out = format!("{whole}.{frac}");
        } else if min_scale > 0 {
            out.push('.');
            out.push_str(&"0".repeat(min_scale as usize));
        }

        if negative && out != "0" {
            format!("-{out}")
        } else {
            out
        }
    }
}

fn ten_pow(n: u32) -> BigInt {
    BigInt::from(10u8).pow(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_collectors_rpc_control::PrepareSourcesResponse;
    use mfm_machine::errors::{ContextError, ErrorInfo, IoError, RunError};
    use mfm_machine::events::DomainEvent;
    use mfm_machine::ids::{ArtifactId, ErrorCode};
    use mfm_machine::io::{IoCall, IoResult};
    use mfm_state_symbol::model::{
        ObservationQuantity, ObservationSource, ObservationValue, ObservationValueSourceRef,
        QuoteCode, SymbolKind, SymbolRole,
    };

    use crate::model::{PortfolioQuoteTotal, PortfolioReport};
    use crate::semantic::{
        AdapterId, ExecutionAnchor, ObservationRuntimeAdapter, RuntimeAdapter,
        SemanticCatalogParts, SubjectRuntimeAdapter, ValuationRuntimeAdapter, ViewRuntimeAdapter,
    };

    #[derive(Default)]
    struct MapContext {
        values: HashMap<String, serde_json::Value>,
    }

    impl DynContext for MapContext {
        fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
            Ok(self.values.get(&key.0).cloned())
        }

        fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
            self.values.insert(key.0, value);
            Ok(())
        }

        fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
            self.values.remove(&key.0);
            Ok(())
        }

        fn dump(&self) -> Result<serde_json::Value, ContextError> {
            let mut out = serde_json::Map::new();
            for (key, value) in &self.values {
                out.insert(key.clone(), value.clone());
            }
            Ok(serde_json::Value::Object(out))
        }
    }

    #[derive(Default)]
    struct NoopRecorder;

    #[async_trait]
    impl EventRecorder for NoopRecorder {
        async fn emit(&mut self, _event: DomainEvent) -> Result<(), RunError> {
            Ok(())
        }

        async fn emit_many(&mut self, _events: Vec<DomainEvent>) -> Result<(), RunError> {
            Ok(())
        }
    }

    struct TestIo {
        prepare_responses: HashMap<(String, String), PrepareSourcesResponse>,
        artifacts: HashMap<String, serde_json::Value>,
        now_millis: u64,
    }

    impl Default for TestIo {
        fn default() -> Self {
            Self {
                prepare_responses: HashMap::new(),
                artifacts: HashMap::new(),
                now_millis: 42,
            }
        }
    }

    #[async_trait]
    impl IoProvider for TestIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            let network_id = call
                .request
                .get("network_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let control_scope = call
                .request
                .get("control_scope")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_CONTROL_SCOPE)
                .to_string();
            let response = self
                .prepare_responses
                .get(&(network_id.clone(), control_scope.clone()))
                .cloned()
                .ok_or_else(|| {
                    IoError::Other(ErrorInfo {
                        code: ErrorCode("missing_prepare_sources_response".to_string()),
                        category: ErrorCategory::Unknown,
                        retryable: false,
                        message: format!(
                            "missing prepare_sources response for network `{network_id}` scope `{control_scope}`"
                        ),
                        details: None,
                    })
                })?;
            Ok(IoResult {
                response: serde_json::to_value(response).expect("prepare response"),
                recorded_payload_id: None,
            })
        }

        async fn record_value(
            &mut self,
            key: FactKey,
            value: serde_json::Value,
        ) -> Result<ArtifactId, IoError> {
            self.artifacts.insert(key.0.clone(), value);
            Ok(ArtifactId(format!("artifact:{}", key.0)))
        }

        async fn get_recorded_fact(
            &mut self,
            key: &FactKey,
        ) -> Result<Option<ArtifactId>, IoError> {
            Ok(self
                .artifacts
                .contains_key(&key.0)
                .then(|| ArtifactId(format!("artifact:{}", key.0))))
        }

        async fn now_millis(&mut self) -> Result<u64, IoError> {
            Ok(self.now_millis)
        }

        async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
            Ok(vec![0; n])
        }
    }

    #[derive(Clone)]
    struct StubSubjectRuntime {
        adapter: AdapterId,
        mismatch_subject_id: Option<String>,
    }

    impl RuntimeAdapter for StubSubjectRuntime {
        fn id(&self) -> &AdapterId {
            &self.adapter
        }
    }

    #[async_trait]
    impl SubjectRuntimeAdapter for StubSubjectRuntime {
        async fn resolve_subject(
            &self,
            _state_id: &StateId,
            _io: &mut dyn IoProvider,
            task: &SubjectResolutionTask,
            _input: SubjectRuntimeInput<'_>,
        ) -> Result<ResolvedSubject, StateError> {
            Ok(ResolvedSubject {
                subject_id: self
                    .mismatch_subject_id
                    .clone()
                    .unwrap_or_else(|| task.subject_id.clone()),
                kind: task.kind.clone(),
                value: serde_json::json!({
                    "network_id": "ethereum-mainnet",
                    "address": "0x000000000000000000000000000000000000dead"
                }),
            })
        }
    }

    #[derive(Clone)]
    struct StubViewRuntime {
        adapter: AdapterId,
    }

    impl RuntimeAdapter for StubViewRuntime {
        fn id(&self) -> &AdapterId {
            &self.adapter
        }
    }

    #[async_trait]
    impl ViewRuntimeAdapter for StubViewRuntime {
        async fn pin_view(
            &self,
            _state_id: &StateId,
            _io: &mut dyn IoProvider,
            task: &ViewPinTask,
            _input: ViewRuntimeInput<'_>,
        ) -> Result<PinnedNetworkView, StateError> {
            Ok(PinnedNetworkView {
                network_view_id: task.network_view_id.clone(),
                network_id: "ethereum-mainnet".to_string(),
                family: task.family.clone(),
                anchor: ExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number: 100,
                },
            })
        }
    }

    #[derive(Clone)]
    struct StubValuationRuntime {
        adapter: AdapterId,
    }

    impl RuntimeAdapter for StubValuationRuntime {
        fn id(&self) -> &AdapterId {
            &self.adapter
        }
    }

    #[async_trait]
    impl ValuationRuntimeAdapter for StubValuationRuntime {
        async fn resolve(
            &self,
            _state_id: &StateId,
            _io: &mut dyn IoProvider,
            task: &ValuationTask,
            _input: ValuationRuntimeInput<'_>,
        ) -> Result<ResolvedUnitPrice, StateError> {
            Ok(ResolvedUnitPrice {
                valuation_id: task.valuation_id.clone(),
                instrument_id: task.instrument_id.clone(),
                priced_symbol_id: task.instrument_id.clone(),
                quote: task.quote,
                unit_price_dec: "2.5".to_string(),
                valuation_reader_kind: "fixed_unit_price".to_string(),
                source_refs: Vec::new(),
            })
        }
    }

    #[derive(Clone)]
    struct StubObservationRuntime {
        adapter: AdapterId,
    }

    impl RuntimeAdapter for StubObservationRuntime {
        fn id(&self) -> &AdapterId {
            &self.adapter
        }
    }

    #[async_trait]
    impl ObservationRuntimeAdapter for StubObservationRuntime {
        async fn observe(
            &self,
            _state_id: &StateId,
            _io: &mut dyn IoProvider,
            _binding: &crate::semantic::CompiledObservationBinding,
            _input: ObservationRuntimeInput<'_>,
        ) -> Result<Observation, StateError> {
            Ok(Observation {
                wallet_id: "wallet_main".to_string(),
                symbol_id: "eth.native.ethereum-mainnet".to_string(),
                display_symbol: Some("ETH".to_string()),
                kind: SymbolKind::NativeBalance,
                role: SymbolRole::Native,
                network_id: "ethereum-mainnet".to_string(),
                protocol: None,
                quantity: ObservationQuantity {
                    raw_dec: "1".to_string(),
                    decimals: 18,
                    amount_dec: "1".to_string(),
                },
                values: vec![ObservationValue {
                    quote: QuoteCode::Usd,
                    priced_symbol_id: "eth.native.ethereum-mainnet".to_string(),
                    value_dec: "2.5".to_string(),
                    unit_price_dec: "2.5".to_string(),
                    valuation_reader_kind: "fixed_unit_price".to_string(),
                    source_refs: vec![ObservationValueSourceRef {
                        source_id: "source-1".to_string(),
                        network_id: "ethereum-mainnet".to_string(),
                        anchor: mfm_state_symbol::model::ObservationAnchor::Evm {
                            chain_id: 1,
                            block_number: 100,
                        },
                    }],
                }],
                source: ObservationSource {
                    balance_reader_kind: "native_balance".to_string(),
                    network_id: "ethereum-mainnet".to_string(),
                    anchor: mfm_state_symbol::model::ObservationAnchor::Evm {
                        chain_id: 1,
                        block_number: 100,
                    },
                },
                metadata: BTreeMap::new(),
            })
        }
    }

    fn sample_portfolio() -> PortfolioConfig {
        serde_json::from_value(serde_json::json!({
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "chain_id": 1u64,
                    "control_scope": "rpc.mainnet",
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_main",
                    "address": "0x000000000000000000000000000000000000dead",
                    "implementation": { "kind": "address_only" },
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": ["eth.native.ethereum-mainnet"],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "2.5"
                                }
                            }
                        ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        }))
        .expect("portfolio")
    }

    fn sample_catalog(mismatch_subject_id: Option<String>) -> Arc<SemanticCatalog> {
        Arc::new(
            SemanticCatalog::new(SemanticCatalogParts {
                subject_runtimes: vec![Arc::new(StubSubjectRuntime {
                    adapter: AdapterId("resolve_subject/evm_address".to_string()),
                    mismatch_subject_id,
                })],
                view_runtimes: vec![Arc::new(StubViewRuntime {
                    adapter: AdapterId("pin_view/evm".to_string()),
                })],
                valuation_runtimes: vec![Arc::new(StubValuationRuntime {
                    adapter: AdapterId("resolve_valuation/fixed_unit_price".to_string()),
                })],
                observation_runtimes: vec![Arc::new(StubObservationRuntime {
                    adapter: AdapterId("observe_position/evm/native_balance".to_string()),
                })],
                ..SemanticCatalogParts::default()
            })
            .expect("catalog"),
        )
    }

    fn sample_prepare_response() -> PrepareSourcesResponse {
        PrepareSourcesResponse {
            control_scope: "rpc.mainnet".to_string(),
            network_id: "ethereum-mainnet".to_string(),
            pool_kind: "test".to_string(),
            available_source_ids: vec!["source-1".to_string()],
            ranked_source_ids: vec!["source-1".to_string()],
            sources: vec![mfm_collectors_rpc_control::PreparedSourceSummary {
                source_id: "source-1".to_string(),
                healthy: true,
                supports_get_proof: None,
                cooldown_until_ms: None,
                last_error_code: None,
            }],
        }
    }

    #[tokio::test]
    async fn fixed_semantic_state_family_executes_compiled_runtime_flow() {
        let mut ctx = MapContext::default();
        let mut io = TestIo::default();
        io.prepare_responses.insert(
            ("ethereum-mainnet".to_string(), "rpc.mainnet".to_string()),
            sample_prepare_response(),
        );
        let mut rec = NoopRecorder;
        let catalog = sample_catalog(None);

        PrepareExecutionSourcesState {
            state_id: StateId::must_new("portfolio.semantic.prepare".to_string()),
            tasks: vec![SourcePreparationTask {
                task_id: "prepare.ethereum-mainnet".to_string(),
                network_view_id: "ethereum-mainnet".to_string(),
                family: NetworkFamily::Evm,
                payload: BTreeMap::from([
                    (
                        "network_id".to_string(),
                        Value::String("ethereum-mainnet".to_string()),
                    ),
                    (
                        "control_scope".to_string(),
                        Value::String("rpc.mainnet".to_string()),
                    ),
                ]),
            }],
            output_key: ContextKey("work.prepared_sources".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("prepare sources");

        ResolveSubjectsState {
            state_id: StateId::must_new("portfolio.semantic.resolve_subjects".to_string()),
            tasks: vec![SubjectResolutionTask {
                task_id: "resolve.wallet_main".to_string(),
                subject_id: "wallet_main".to_string(),
                kind: SubjectKind::EvmAddress,
                adapter: AdapterId("resolve_subject/evm_address".to_string()),
                payload: BTreeMap::new(),
            }],
            catalog: Arc::clone(&catalog),
            prepared_sources_key: ContextKey("work.prepared_sources".to_string()),
            output_key: ContextKey("work.resolved_subjects".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("resolve subjects");

        PinExecutionViewsState {
            state_id: StateId::must_new("portfolio.semantic.pin_views".to_string()),
            tasks: vec![ViewPinTask {
                task_id: "pin.ethereum-mainnet".to_string(),
                network_view_id: "ethereum-mainnet".to_string(),
                family: NetworkFamily::Evm,
                adapter: AdapterId("pin_view/evm".to_string()),
                payload: BTreeMap::new(),
            }],
            catalog: Arc::clone(&catalog),
            prepared_sources_key: ContextKey("work.prepared_sources".to_string()),
            output_key: ContextKey("work.pinned_views".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("pin views");

        ResolveValuationInputsState {
            state_id: StateId::must_new("portfolio.semantic.resolve_values".to_string()),
            tasks: vec![ValuationTask {
                valuation_id: "eth.native.ethereum-mainnet.quote.usd".to_string(),
                instrument_id: "eth.native.ethereum-mainnet".to_string(),
                quote: QuoteCode::Usd,
                adapter: AdapterId("resolve_valuation/fixed_unit_price".to_string()),
                payload: BTreeMap::new(),
            }],
            catalog: Arc::clone(&catalog),
            pinned_views_key: ContextKey("work.pinned_views".to_string()),
            output_key: ContextKey("work.resolved_valuations".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("resolve values");

        ObserveCompiledBatchState {
            state_id: StateId::must_new("portfolio.semantic.observe_batch".to_string()),
            batch: CompiledObservationBatch {
                batch_id: "observe.ethereum-mainnet.native".to_string(),
                adapter: AdapterId("observe_position/evm/native_balance".to_string()),
                network_view_id: "ethereum-mainnet".to_string(),
                bindings: vec![crate::semantic::CompiledObservationBinding {
                    binding_id: "binding.wallet_main.eth".to_string(),
                    observation_key: crate::semantic::ObservationKey {
                        subject_id: "wallet_main".to_string(),
                        network_view_id: "ethereum-mainnet".to_string(),
                        instrument_id: "eth.native.ethereum-mainnet".to_string(),
                        position_kind: crate::semantic::PositionSemantics::SpotBalance,
                        venue_id: None,
                        discriminator: Some("eth.native.ethereum-mainnet".to_string()),
                    },
                    adapter: AdapterId("observe_position/evm/native_balance".to_string()),
                    valuation_ids: vec!["eth.native.ethereum-mainnet.quote.usd".to_string()],
                    payload: BTreeMap::new(),
                }],
            },
            catalog,
            resolved_subjects_key: ContextKey("work.resolved_subjects".to_string()),
            pinned_views_key: ContextKey("work.pinned_views".to_string()),
            resolved_valuations_key: ContextKey("work.resolved_valuations".to_string()),
            output_key: ContextKey("work.batch_observations".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("observe batch");

        MergeObservationsState {
            state_id: StateId::must_new("portfolio.semantic.merge".to_string()),
            input_keys: vec![ContextKey("work.batch_observations".to_string())],
            output_key: ContextKey("work.observations".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("merge observations");

        AssembleSnapshotState {
            state_id: StateId::must_new("portfolio.semantic.snapshot".to_string()),
            portfolio: sample_portfolio(),
            resolved_subjects_key: ContextKey("work.resolved_subjects".to_string()),
            pinned_views_key: ContextKey("work.pinned_views".to_string()),
            observations_key: ContextKey("work.observations".to_string()),
            fact_key: FactKey("portfolio:semantic_snapshot".to_string()),
            artifact_id_output_key: ContextKey("out.snapshot_artifact_id".to_string()),
            snapshot_output_key: ContextKey("out.snapshot".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("assemble snapshot");

        ProjectReportState {
            state_id: StateId::must_new("portfolio.semantic.report".to_string()),
            snapshot_key: ContextKey("out.snapshot".to_string()),
            output_key: ContextKey("out.report".to_string()),
            event_name: "portfolio_tracker.completed",
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("project report");

        let report: PortfolioReport = serde_json::from_value(
            ctx.read(&ContextKey("out.report".to_string()))
                .expect("read")
                .expect("report"),
        )
        .expect("typed report");
        assert_eq!(report.portfolio_id, "portfolio_main");
        assert_eq!(report.error_count, 0);
        assert_eq!(
            report.totals_by_quote,
            vec![PortfolioQuoteTotal {
                quote: QuoteCode::Usd,
                assets_value_dec: "2.5".to_string(),
                collateral_value_dec: "0".to_string(),
                debt_value_dec: "0".to_string(),
                staked_value_dec: "0".to_string(),
                net_value_dec: "2.5".to_string(),
            }]
        );

        let snapshot: PortfolioSnapshot = serde_json::from_value(
            ctx.read(&ContextKey("out.snapshot".to_string()))
                .expect("read")
                .expect("snapshot"),
        )
        .expect("typed snapshot");
        assert_eq!(snapshot.wallets.len(), 1);
        assert_eq!(
            snapshot.wallets[0].address,
            "0x000000000000000000000000000000000000dead"
        );
        assert_eq!(snapshot.network_pins.len(), 1);
        assert_eq!(snapshot.schema_version, 2);
        assert_eq!(
            snapshot.network_pins[0].anchor,
            SnapshotExecutionAnchor::Evm {
                chain_id: 1,
                block_number: 100,
            }
        );
    }

    #[tokio::test]
    async fn resolve_subjects_fails_fast_on_runtime_identity_mismatch() {
        let mut ctx = MapContext::default();
        ctx.write(
            ContextKey("work.prepared_sources".to_string()),
            serde_json::json!({}),
        )
        .expect("write");
        let mut io = TestIo::default();
        let mut rec = NoopRecorder;

        let err = ResolveSubjectsState {
            state_id: StateId::must_new("portfolio.semantic.resolve_subjects".to_string()),
            tasks: vec![SubjectResolutionTask {
                task_id: "resolve.wallet_main".to_string(),
                subject_id: "wallet_main".to_string(),
                kind: SubjectKind::EvmAddress,
                adapter: AdapterId("resolve_subject/evm_address".to_string()),
                payload: BTreeMap::new(),
            }],
            catalog: sample_catalog(Some("wallet_other".to_string())),
            prepared_sources_key: ContextKey("work.prepared_sources".to_string()),
            output_key: ContextKey("work.resolved_subjects".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect_err("must fail");

        assert_eq!(err.info.code.0, "subject_runtime_identity_mismatch");
    }

    #[tokio::test]
    async fn assemble_snapshot_rejects_multiple_views_for_one_v1_network_pin() {
        let mut ctx = MapContext::default();
        let mut io = TestIo::default();
        let mut rec = NoopRecorder;
        ctx.write(
            ContextKey("work.resolved_subjects".to_string()),
            serde_json::json!({
                "wallet_main": {
                    "subject_id": "wallet_main",
                    "kind": "evm_address",
                    "value": {
                        "network_id": "ethereum-mainnet",
                        "address": "0x000000000000000000000000000000000000dead"
                    }
                }
            }),
        )
        .expect("write subjects");
        ctx.write(
            ContextKey("work.pinned_views".to_string()),
            serde_json::json!({
                "view_a": {
                    "network_view_id": "view_a",
                    "network_id": "ethereum-mainnet",
                    "family": "evm",
                    "anchor": {
                        "family": "evm",
                        "chain_id": 1u64,
                        "block_number": 100u64
                    }
                },
                "view_b": {
                    "network_view_id": "view_b",
                    "network_id": "ethereum-mainnet",
                    "family": "evm",
                    "anchor": {
                        "family": "evm",
                        "chain_id": 1u64,
                        "block_number": 101u64
                    }
                }
            }),
        )
        .expect("write views");
        ctx.write(
            ContextKey("work.observations".to_string()),
            serde_json::json!([]),
        )
        .expect("write observations");

        let err = AssembleSnapshotState {
            state_id: StateId::must_new("portfolio.semantic.snapshot".to_string()),
            portfolio: sample_portfolio(),
            resolved_subjects_key: ContextKey("work.resolved_subjects".to_string()),
            pinned_views_key: ContextKey("work.pinned_views".to_string()),
            observations_key: ContextKey("work.observations".to_string()),
            fact_key: FactKey("portfolio:semantic_snapshot".to_string()),
            artifact_id_output_key: ContextKey("out.snapshot_artifact_id".to_string()),
            snapshot_output_key: ContextKey("out.snapshot".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect_err("must fail");

        assert_eq!(err.info.code.0, "duplicate_snapshot_network_pin");
    }

    #[tokio::test]
    async fn merge_observations_orders_inputs_canonically() {
        let mut ctx = MapContext::default();
        ctx.write(
            ContextKey("work.observations.a".to_string()),
            serde_json::json!([observation_with_value(
                "wallet_b",
                "eth.native.ethereum-mainnet",
                SymbolRole::Native,
                QuoteCode::Usd,
                "2.5"
            ),]),
        )
        .expect("write");
        ctx.write(
            ContextKey("work.observations.b".to_string()),
            serde_json::json!([
                observation_with_value(
                    "wallet_a",
                    "usdc.wallet.ethereum-mainnet",
                    SymbolRole::Asset,
                    QuoteCode::Usd,
                    "1.0"
                ),
                observation_with_value(
                    "wallet_a",
                    "eth.native.ethereum-mainnet",
                    SymbolRole::Native,
                    QuoteCode::Usd,
                    "3.0"
                ),
            ]),
        )
        .expect("write");

        MergeObservationsState {
            state_id: StateId::must_new("portfolio.semantic.merge".to_string()),
            input_keys: vec![
                ContextKey("work.observations.a".to_string()),
                ContextKey("work.observations.b".to_string()),
            ],
            output_key: ContextKey("work.observations".to_string()),
        }
        .handle(&mut ctx, &mut TestIo::default(), &mut NoopRecorder)
        .await
        .expect("merge");

        let observations: Vec<Observation> = serde_json::from_value(
            ctx.read(&ContextKey("work.observations".to_string()))
                .expect("read")
                .expect("observations"),
        )
        .expect("typed");
        assert_eq!(observations.len(), 3);
        assert_eq!(observations[0].wallet_id, "wallet_a");
        assert_eq!(observations[0].symbol_id, "eth.native.ethereum-mainnet");
        assert_eq!(observations[1].wallet_id, "wallet_a");
        assert_eq!(observations[1].symbol_id, "usdc.wallet.ethereum-mainnet");
        assert_eq!(observations[2].wallet_id, "wallet_b");
        assert_eq!(observations[2].symbol_id, "eth.native.ethereum-mainnet");
    }

    #[tokio::test]
    async fn project_report_derives_net_exposure_buckets() {
        let snapshot = PortfolioSnapshot {
            schema_version: 2,
            portfolio_id: "portfolio_roles".to_string(),
            generated_at_ms: 1234,
            network_pins: vec![NetworkPin {
                network_id: "ethereum-mainnet".to_string(),
                anchor: SnapshotExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number: 100,
                },
            }],
            wallets: vec![WalletSnapshot {
                wallet_id: "wallet_main".to_string(),
                address: "0x000000000000000000000000000000000000dead".to_string(),
                subject_kind: WalletSubjectKind::EvmAddress,
                network_id: "ethereum-mainnet".to_string(),
                observations: vec![
                    observation_with_value(
                        "wallet_main",
                        "asset.main",
                        SymbolRole::Asset,
                        QuoteCode::Usd,
                        "12.50",
                    ),
                    observation_with_value(
                        "wallet_main",
                        "collateral.main",
                        SymbolRole::Collateral,
                        QuoteCode::Usd,
                        "7.25",
                    ),
                    observation_with_value(
                        "wallet_main",
                        "debt.main",
                        SymbolRole::Debt,
                        QuoteCode::Usd,
                        "30.00",
                    ),
                    observation_with_value(
                        "wallet_main",
                        "staked.main",
                        SymbolRole::Staked,
                        QuoteCode::Usd,
                        "1.25",
                    ),
                ],
            }],
            symbol_configs: vec![],
            errors: vec![],
        };

        let mut ctx = MapContext::default();
        ctx.write(
            ContextKey("snapshot".to_string()),
            serde_json::to_value(&snapshot).expect("snapshot json"),
        )
        .expect("write snapshot");

        ProjectReportState {
            state_id: StateId::must_new("portfolio.semantic.report".to_string()),
            snapshot_key: ContextKey("snapshot".to_string()),
            output_key: ContextKey("report".to_string()),
            event_name: "portfolio_tracker.completed",
        }
        .handle(&mut ctx, &mut TestIo::default(), &mut NoopRecorder)
        .await
        .expect("project report");

        let report: PortfolioReport = serde_json::from_value(
            ctx.read(&ContextKey("report".to_string()))
                .expect("read")
                .expect("report"),
        )
        .expect("typed report");
        let wallet_usd =
            find_quote_total(&report.wallet_summaries[0].totals_by_quote, QuoteCode::Usd);
        assert_eq!(wallet_usd.assets_value_dec, "12.50");
        assert_eq!(wallet_usd.collateral_value_dec, "7.25");
        assert_eq!(wallet_usd.debt_value_dec, "30.00");
        assert_eq!(wallet_usd.staked_value_dec, "1.25");
        assert_eq!(wallet_usd.net_value_dec, "-9.00");

        let portfolio_usd = find_quote_total(&report.totals_by_quote, QuoteCode::Usd);
        assert_eq!(portfolio_usd, wallet_usd);
    }

    fn observation_with_value(
        wallet_id: &str,
        symbol_id: &str,
        role: SymbolRole,
        quote: QuoteCode,
        value_dec: &str,
    ) -> Observation {
        Observation {
            wallet_id: wallet_id.to_string(),
            symbol_id: symbol_id.to_string(),
            display_symbol: None,
            kind: SymbolKind::NativeBalance,
            role,
            network_id: "ethereum-mainnet".to_string(),
            protocol: None,
            quantity: ObservationQuantity {
                raw_dec: "1".to_string(),
                decimals: 18,
                amount_dec: "1".to_string(),
            },
            values: vec![ObservationValue {
                quote,
                priced_symbol_id: symbol_id.to_string(),
                value_dec: value_dec.to_string(),
                unit_price_dec: value_dec.to_string(),
                valuation_reader_kind: "fixed_unit_price".to_string(),
                source_refs: vec![],
            }],
            source: ObservationSource {
                balance_reader_kind: "native_balance".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                anchor: mfm_state_symbol::model::ObservationAnchor::Evm {
                    chain_id: 1,
                    block_number: 100,
                },
            },
            metadata: BTreeMap::new(),
        }
    }

    fn find_quote_total(totals: &[PortfolioQuoteTotal], quote: QuoteCode) -> PortfolioQuoteTotal {
        totals
            .iter()
            .find(|total| total.quote == quote)
            .cloned()
            .expect("quote total")
    }
}
