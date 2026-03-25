use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use mfm_machine::errors::StateError;
use mfm_machine::hashing::{canonical_json_bytes, CanonicalJsonError};
use mfm_machine::ids::StateId;
use mfm_machine::io::IoProvider;
use mfm_state_symbol::model::ObservationValueSourceRef;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::{
    json_object_value, AdapterId, CompiledObservationBinding, NetworkFamily, NetworkView,
    Observation, ObservationTarget, Position, QuoteCode, Subject, SubjectKind, Valuation,
};

/// Runtime-resolved subject value selected for semantic execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedSubject {
    /// Stable semantic subject identifier.
    pub subject_id: String,
    /// Semantic subject family.
    pub kind: SubjectKind,
    /// Adapter-owned resolved subject payload.
    pub value: Value,
}

/// Pinned execution anchor selected for one network view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum ExecutionAnchor {
    /// EVM-family pinned execution anchor.
    Evm {
        /// Expected chain id for the pinned view.
        chain_id: u64,
        /// Concrete pinned block number.
        block_number: u64,
    },
    /// Bitcoin-family pinned execution anchor.
    Bitcoin {
        /// Concrete pinned block height.
        height: u64,
        /// Concrete pinned block hash.
        block_hash: String,
    },
}

/// Runtime-resolved pinned network view used by semantic execution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedNetworkView {
    /// Stable semantic execution-view identifier.
    pub network_view_id: String,
    /// Stable user-authored network identifier.
    pub network_id: String,
    /// Semantic network family.
    pub family: NetworkFamily,
    /// Concrete replayable execution anchor.
    pub anchor: ExecutionAnchor,
}

/// Runtime-resolved valuation output used by observation execution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedUnitPrice {
    /// Stable valuation identifier.
    pub valuation_id: String,
    /// Instrument valued by this result.
    pub instrument_id: String,
    /// Symbol identity whose unit price was resolved.
    pub priced_symbol_id: String,
    /// Quote unit emitted by the valuation.
    pub quote: QuoteCode,
    /// Canonical decimal unit price.
    pub unit_price_dec: String,
    /// Stable valuation reader kind used for read-model projection.
    pub valuation_reader_kind: String,
    /// Concrete source refs used for this valuation.
    pub source_refs: Vec<ObservationValueSourceRef>,
}

/// Planner request for compiling one observation binding.
pub struct ObservationPlanRequest<'a> {
    /// Semantic observation target being compiled.
    pub target: &'a ObservationTarget,
    /// Semantic subject referenced by the target.
    pub subject: &'a Subject,
    /// Semantic network view referenced by the target.
    pub network_view: &'a NetworkView,
    /// Semantic position referenced by the target.
    pub position: &'a Position,
    /// Semantic instrument referenced by the position.
    pub instrument: &'a super::Instrument,
    /// Optional semantic venue referenced by the position.
    pub venue: Option<&'a super::Venue>,
    /// Valuations attached to the target in deterministic planner order.
    pub valuations: &'a [Valuation],
}

/// Planner request for compiling one valuation task.
pub struct ValuationPlanRequest<'a> {
    /// Semantic valuation being compiled.
    pub valuation: &'a Valuation,
    /// Semantic instrument referenced by the valuation.
    pub instrument: &'a super::Instrument,
}

/// Planner request for compiling one subject-resolution task.
pub struct SubjectPlanRequest<'a> {
    /// Semantic subject being compiled.
    pub subject: &'a Subject,
}

/// Planner request for compiling one view-pin task.
pub struct ViewPlanRequest<'a> {
    /// Semantic execution view being compiled.
    pub network_view: &'a NetworkView,
}

/// Runtime inputs available to subject-resolution adapters.
pub struct SubjectRuntimeInput<'a> {
    /// Prepared source payloads keyed by semantic network view id or other planner-owned task ids.
    pub prepared_sources: &'a BTreeMap<String, Value>,
}

/// Runtime inputs available to view-pin adapters.
pub struct ViewRuntimeInput<'a> {
    /// Prepared source payloads keyed by semantic network view id or other planner-owned task ids.
    pub prepared_sources: &'a BTreeMap<String, Value>,
}

/// Runtime inputs available to valuation adapters.
pub struct ValuationRuntimeInput<'a> {
    /// Pinned execution views available to valuation execution.
    pub pinned_views: &'a BTreeMap<String, PinnedNetworkView>,
}

/// Runtime inputs available to observation adapters.
pub struct ObservationRuntimeInput<'a> {
    /// Runtime-resolved subjects keyed by semantic subject id.
    pub resolved_subjects: &'a BTreeMap<String, ResolvedSubject>,
    /// Pinned execution views keyed by semantic execution-view id.
    pub pinned_views: &'a BTreeMap<String, PinnedNetworkView>,
    /// Resolved valuation outputs keyed by valuation id.
    pub resolved_valuations: &'a BTreeMap<String, ResolvedUnitPrice>,
}

/// Base trait for pure planner-side semantic adapters.
pub trait PlannerAdapter: Send + Sync {
    /// Returns the stable planner-selected adapter id.
    fn id(&self) -> AdapterId;
}

/// Planner adapter that compiles observation bindings.
pub trait ObservationPlannerAdapter: PlannerAdapter {
    /// Returns true when this adapter can compile the provided observation request.
    fn supports(&self, req: &ObservationPlanRequest<'_>) -> bool;

    /// Compiles one observation binding from semantic input.
    fn plan(
        &self,
        req: ObservationPlanRequest<'_>,
    ) -> Result<CompiledObservationBinding, PlanningError>;
}

/// Planner adapter that compiles valuation tasks.
pub trait ValuationPlannerAdapter: PlannerAdapter {
    /// Returns true when this adapter can compile the provided valuation request.
    fn supports(&self, req: &ValuationPlanRequest<'_>) -> bool;

    /// Compiles one valuation task from semantic input.
    fn plan(&self, req: ValuationPlanRequest<'_>) -> Result<super::ValuationTask, PlanningError>;
}

/// Planner adapter that compiles subject-resolution tasks.
pub trait SubjectPlannerAdapter: PlannerAdapter {
    /// Returns true when this adapter can compile the provided subject request.
    fn supports(&self, req: &SubjectPlanRequest<'_>) -> bool;

    /// Compiles one subject-resolution task from semantic input.
    fn plan(
        &self,
        req: SubjectPlanRequest<'_>,
    ) -> Result<super::SubjectResolutionTask, PlanningError>;
}

/// Planner adapter that compiles view-pin tasks.
pub trait ViewPlannerAdapter: PlannerAdapter {
    /// Returns true when this adapter can compile the provided view-pin request.
    fn supports(&self, req: &ViewPlanRequest<'_>) -> bool;

    /// Compiles one view-pin task from semantic input.
    fn plan(&self, req: ViewPlanRequest<'_>) -> Result<super::ViewPinTask, PlanningError>;
}

/// Base trait for runtime semantic adapters keyed by planned adapter id.
pub trait RuntimeAdapter: Send + Sync {
    /// Returns the stable runtime adapter id.
    fn id(&self) -> &AdapterId;
}

/// Runtime adapter that resolves semantic subjects.
#[async_trait]
pub trait SubjectRuntimeAdapter: RuntimeAdapter {
    /// Resolves one semantic subject into a runtime subject payload.
    async fn resolve_subject(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        task: &super::SubjectResolutionTask,
        input: SubjectRuntimeInput<'_>,
    ) -> Result<ResolvedSubject, StateError>;
}

/// Runtime adapter that pins semantic network views.
#[async_trait]
pub trait ViewRuntimeAdapter: RuntimeAdapter {
    /// Pins one semantic execution view into a replayable runtime anchor.
    async fn pin_view(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        task: &super::ViewPinTask,
        input: ViewRuntimeInput<'_>,
    ) -> Result<PinnedNetworkView, StateError>;
}

/// Runtime adapter that resolves semantic valuations.
#[async_trait]
pub trait ValuationRuntimeAdapter: RuntimeAdapter {
    /// Resolves one planner-owned valuation task into a concrete unit price.
    async fn resolve(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        task: &super::ValuationTask,
        input: ValuationRuntimeInput<'_>,
    ) -> Result<ResolvedUnitPrice, StateError>;
}

/// Runtime adapter that observes one compiled semantic binding.
#[async_trait]
pub trait ObservationRuntimeAdapter: RuntimeAdapter {
    /// Executes one compiled observation binding and returns the canonical observation read model.
    async fn observe(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        binding: &CompiledObservationBinding,
        input: ObservationRuntimeInput<'_>,
    ) -> Result<Observation, StateError>;
}

/// Pure planner errors surfaced by semantic adapter selection and compilation.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PlanningError {
    /// No planner adapter matched the semantic request.
    #[error("no {kind} planner adapter matched the request")]
    NoMatchingAdapter {
        /// Semantic task kind being planned.
        kind: &'static str,
    },
    /// More than one planner adapter matched the semantic request.
    #[error("multiple {kind} planner adapters matched the request: {adapters:?}")]
    AmbiguousAdapterMatch {
        /// Semantic task kind being planned.
        kind: &'static str,
        /// Matching adapter ids in sorted order.
        adapters: Vec<AdapterId>,
    },
    /// A planner adapter emitted a task or binding for a different adapter id than itself.
    #[error(
        "{kind} planner adapter `{planner_adapter}` emitted task or binding for `{emitted_adapter}`"
    )]
    EmittedAdapterIdMismatch {
        /// Semantic task kind being planned.
        kind: &'static str,
        /// Planner adapter chosen by selection.
        planner_adapter: AdapterId,
        /// Adapter id emitted in the planned task or binding.
        emitted_adapter: AdapterId,
    },
    /// A planner adapter emitted a payload that is not canonical-json-safe.
    #[error("{kind} planner adapter `{adapter}` emitted non-canonical payload: {reason}")]
    InvalidPlannedPayload {
        /// Semantic task kind being planned.
        kind: &'static str,
        /// Adapter that emitted the invalid payload.
        adapter: AdapterId,
        /// Underlying canonical-json failure.
        reason: CanonicalJsonError,
    },
    /// A planner adapter emitted an empty required identifier.
    #[error("{kind} planner adapter emitted empty `{field}`")]
    EmptyPlannedId {
        /// Semantic task kind being planned.
        kind: &'static str,
        /// Field that was empty.
        field: &'static str,
    },
    /// Compiler-owned planning failure not attributable to one adapter implementation.
    #[error("{code}: {message}")]
    Compile {
        /// Stable compiler error code.
        code: &'static str,
        /// Safe human-readable message.
        message: String,
    },
    /// Adapter-owned planner failure.
    #[error("{adapter}: {message}")]
    Adapter {
        /// Adapter reporting the failure.
        adapter: AdapterId,
        /// Stable planner error code.
        code: &'static str,
        /// Safe human-readable message.
        message: String,
    },
}

impl PlanningError {
    /// Creates a planner error reported by one concrete adapter.
    pub fn adapter(adapter: AdapterId, code: &'static str, message: impl Into<String>) -> Self {
        Self::Adapter {
            adapter,
            code,
            message: message.into(),
        }
    }

    /// Creates a compiler-owned planning error.
    pub fn compile(code: &'static str, message: impl Into<String>) -> Self {
        Self::Compile {
            code,
            message: message.into(),
        }
    }
}

/// Catalog-construction and runtime-lookup errors for semantic adapters.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SemanticCatalogError {
    /// Duplicate planner adapter ids are forbidden because selection must remain deterministic.
    #[error("duplicate {kind} planner adapter `{adapter}`")]
    DuplicatePlannerAdapter {
        /// Semantic task kind.
        kind: &'static str,
        /// Duplicate planner adapter id.
        adapter: AdapterId,
    },
    /// Duplicate runtime adapter ids are forbidden because runtime lookup is by exact id.
    #[error("duplicate {kind} runtime adapter `{adapter}`")]
    DuplicateRuntimeAdapter {
        /// Semantic task kind.
        kind: &'static str,
        /// Duplicate runtime adapter id.
        adapter: AdapterId,
    },
    /// Runtime lookup failed because no adapter was registered for the planned adapter id.
    #[error("unknown {kind} runtime adapter `{adapter}`")]
    UnknownRuntimeAdapter {
        /// Semantic task kind.
        kind: &'static str,
        /// Planned adapter id that could not be resolved.
        adapter: AdapterId,
    },
}

/// Deterministic semantic catalog inputs.
#[derive(Default)]
pub struct SemanticCatalogParts {
    /// Planner adapters for observation compilation.
    pub observation_planners: Vec<Arc<dyn ObservationPlannerAdapter>>,
    /// Runtime adapters for compiled observation execution.
    pub observation_runtimes: Vec<Arc<dyn ObservationRuntimeAdapter>>,
    /// Planner adapters for valuation compilation.
    pub valuation_planners: Vec<Arc<dyn ValuationPlannerAdapter>>,
    /// Runtime adapters for valuation execution.
    pub valuation_runtimes: Vec<Arc<dyn ValuationRuntimeAdapter>>,
    /// Planner adapters for subject-resolution compilation.
    pub subject_planners: Vec<Arc<dyn SubjectPlannerAdapter>>,
    /// Runtime adapters for subject resolution.
    pub subject_runtimes: Vec<Arc<dyn SubjectRuntimeAdapter>>,
    /// Planner adapters for execution-view pin compilation.
    pub view_planners: Vec<Arc<dyn ViewPlannerAdapter>>,
    /// Runtime adapters for execution-view pinning.
    pub view_runtimes: Vec<Arc<dyn ViewRuntimeAdapter>>,
}

/// Deterministic semantic adapter catalog used by the compiler and runtime.
pub struct SemanticCatalog {
    observation_planners: Vec<Arc<dyn ObservationPlannerAdapter>>,
    observation_runtimes: BTreeMap<AdapterId, Arc<dyn ObservationRuntimeAdapter>>,
    valuation_planners: Vec<Arc<dyn ValuationPlannerAdapter>>,
    valuation_runtimes: BTreeMap<AdapterId, Arc<dyn ValuationRuntimeAdapter>>,
    subject_planners: Vec<Arc<dyn SubjectPlannerAdapter>>,
    subject_runtimes: BTreeMap<AdapterId, Arc<dyn SubjectRuntimeAdapter>>,
    view_planners: Vec<Arc<dyn ViewPlannerAdapter>>,
    view_runtimes: BTreeMap<AdapterId, Arc<dyn ViewRuntimeAdapter>>,
}

impl SemanticCatalog {
    /// Builds a deterministic semantic catalog and rejects duplicate adapter ids.
    pub fn new(parts: SemanticCatalogParts) -> Result<Self, SemanticCatalogError> {
        ensure_unique_planner_ids("observation", &parts.observation_planners)?;
        ensure_unique_planner_ids("valuation", &parts.valuation_planners)?;
        ensure_unique_planner_ids("subject", &parts.subject_planners)?;
        ensure_unique_planner_ids("view", &parts.view_planners)?;

        Ok(Self {
            observation_planners: parts.observation_planners,
            observation_runtimes: runtime_map("observation", parts.observation_runtimes)?,
            valuation_planners: parts.valuation_planners,
            valuation_runtimes: runtime_map("valuation", parts.valuation_runtimes)?,
            subject_planners: parts.subject_planners,
            subject_runtimes: runtime_map("subject", parts.subject_runtimes)?,
            view_planners: parts.view_planners,
            view_runtimes: runtime_map("view", parts.view_runtimes)?,
        })
    }

    /// Plans one observation binding with exact-one adapter selection.
    pub fn plan_observation(
        &self,
        req: ObservationPlanRequest<'_>,
    ) -> Result<CompiledObservationBinding, PlanningError> {
        let planner = select_planner("observation", &self.observation_planners, |adapter| {
            adapter.supports(&req)
        })?;
        let planner_id = planner.id();
        let mut binding = planner.plan(req)?;
        ensure_non_empty_planned_id("observation", "binding_id", &binding.binding_id)?;
        ensure_non_empty_planned_id("observation", "adapter", &binding.adapter.0)?;
        ensure_matching_emitted_adapter("observation", &planner_id, &binding.adapter)?;
        binding.normalize();
        ensure_canonical_planned_payload("observation", &planner_id, &binding.payload)?;
        Ok(binding)
    }

    /// Plans one valuation task with exact-one adapter selection.
    pub fn plan_valuation(
        &self,
        req: ValuationPlanRequest<'_>,
    ) -> Result<super::ValuationTask, PlanningError> {
        let planner = select_planner("valuation", &self.valuation_planners, |adapter| {
            adapter.supports(&req)
        })?;
        let planner_id = planner.id();
        let task = planner.plan(req)?;
        ensure_non_empty_planned_id("valuation", "valuation_id", &task.valuation_id)?;
        ensure_non_empty_planned_id("valuation", "adapter", &task.adapter.0)?;
        ensure_matching_emitted_adapter("valuation", &planner_id, &task.adapter)?;
        ensure_canonical_planned_payload("valuation", &planner_id, &task.payload)?;
        Ok(task)
    }

    /// Plans one subject-resolution task with exact-one adapter selection.
    pub fn plan_subject(
        &self,
        req: SubjectPlanRequest<'_>,
    ) -> Result<super::SubjectResolutionTask, PlanningError> {
        let planner = select_planner("subject", &self.subject_planners, |adapter| {
            adapter.supports(&req)
        })?;
        let planner_id = planner.id();
        let task = planner.plan(req)?;
        ensure_non_empty_planned_id("subject", "task_id", &task.task_id)?;
        ensure_non_empty_planned_id("subject", "adapter", &task.adapter.0)?;
        ensure_matching_emitted_adapter("subject", &planner_id, &task.adapter)?;
        ensure_canonical_planned_payload("subject", &planner_id, &task.payload)?;
        Ok(task)
    }

    /// Plans one execution-view pin task with exact-one adapter selection.
    pub fn plan_view(&self, req: ViewPlanRequest<'_>) -> Result<super::ViewPinTask, PlanningError> {
        let planner = select_planner("view", &self.view_planners, |adapter| {
            adapter.supports(&req)
        })?;
        let planner_id = planner.id();
        let task = planner.plan(req)?;
        ensure_non_empty_planned_id("view", "task_id", &task.task_id)?;
        ensure_non_empty_planned_id("view", "adapter", &task.adapter.0)?;
        ensure_matching_emitted_adapter("view", &planner_id, &task.adapter)?;
        ensure_canonical_planned_payload("view", &planner_id, &task.payload)?;
        Ok(task)
    }

    /// Resolves one observation runtime adapter by its planned adapter id.
    pub fn observation_runtime(
        &self,
        adapter: &AdapterId,
    ) -> Result<Arc<dyn ObservationRuntimeAdapter>, SemanticCatalogError> {
        self.observation_runtimes
            .get(adapter)
            .cloned()
            .ok_or_else(|| SemanticCatalogError::UnknownRuntimeAdapter {
                kind: "observation",
                adapter: adapter.clone(),
            })
    }

    /// Resolves one valuation runtime adapter by its planned adapter id.
    pub fn valuation_runtime(
        &self,
        adapter: &AdapterId,
    ) -> Result<Arc<dyn ValuationRuntimeAdapter>, SemanticCatalogError> {
        self.valuation_runtimes
            .get(adapter)
            .cloned()
            .ok_or_else(|| SemanticCatalogError::UnknownRuntimeAdapter {
                kind: "valuation",
                adapter: adapter.clone(),
            })
    }

    /// Resolves one subject runtime adapter by its planned adapter id.
    pub fn subject_runtime(
        &self,
        adapter: &AdapterId,
    ) -> Result<Arc<dyn SubjectRuntimeAdapter>, SemanticCatalogError> {
        self.subject_runtimes.get(adapter).cloned().ok_or_else(|| {
            SemanticCatalogError::UnknownRuntimeAdapter {
                kind: "subject",
                adapter: adapter.clone(),
            }
        })
    }

    /// Resolves one view runtime adapter by its planned adapter id.
    pub fn view_runtime(
        &self,
        adapter: &AdapterId,
    ) -> Result<Arc<dyn ViewRuntimeAdapter>, SemanticCatalogError> {
        self.view_runtimes.get(adapter).cloned().ok_or_else(|| {
            SemanticCatalogError::UnknownRuntimeAdapter {
                kind: "view",
                adapter: adapter.clone(),
            }
        })
    }
}

fn ensure_unique_planner_ids<A: PlannerAdapter + ?Sized>(
    kind: &'static str,
    planners: &[Arc<A>],
) -> Result<(), SemanticCatalogError> {
    let mut seen = BTreeSet::new();
    for planner in planners {
        let adapter = planner.id();
        if !seen.insert(adapter.clone()) {
            return Err(SemanticCatalogError::DuplicatePlannerAdapter { kind, adapter });
        }
    }
    Ok(())
}

fn runtime_map<A: RuntimeAdapter + ?Sized>(
    kind: &'static str,
    adapters: Vec<Arc<A>>,
) -> Result<BTreeMap<AdapterId, Arc<A>>, SemanticCatalogError> {
    let mut by_id = BTreeMap::new();
    for adapter in adapters {
        let adapter_id = adapter.id().clone();
        if by_id
            .insert(adapter_id.clone(), Arc::clone(&adapter))
            .is_some()
        {
            return Err(SemanticCatalogError::DuplicateRuntimeAdapter {
                kind,
                adapter: adapter_id,
            });
        }
    }
    Ok(by_id)
}

fn select_planner<A, F>(
    kind: &'static str,
    planners: &[Arc<A>],
    supports: F,
) -> Result<Arc<A>, PlanningError>
where
    A: PlannerAdapter + ?Sized + 'static,
    F: Fn(&A) -> bool,
{
    let mut matches: Vec<Arc<A>> = planners
        .iter()
        .filter(|planner| supports(planner.as_ref()))
        .cloned()
        .collect();
    match matches.len() {
        0 => Err(PlanningError::NoMatchingAdapter { kind }),
        1 => Ok(matches.pop().expect("one match")),
        _ => {
            let mut adapters: Vec<AdapterId> = matches.iter().map(|planner| planner.id()).collect();
            adapters.sort();
            Err(PlanningError::AmbiguousAdapterMatch { kind, adapters })
        }
    }
}

fn ensure_non_empty_planned_id(
    kind: &'static str,
    field: &'static str,
    value: &str,
) -> Result<(), PlanningError> {
    if value.is_empty() {
        return Err(PlanningError::EmptyPlannedId { kind, field });
    }
    Ok(())
}

fn ensure_matching_emitted_adapter(
    kind: &'static str,
    planner_adapter: &AdapterId,
    emitted_adapter: &AdapterId,
) -> Result<(), PlanningError> {
    if planner_adapter != emitted_adapter {
        return Err(PlanningError::EmittedAdapterIdMismatch {
            kind,
            planner_adapter: planner_adapter.clone(),
            emitted_adapter: emitted_adapter.clone(),
        });
    }
    Ok(())
}

fn ensure_canonical_planned_payload(
    kind: &'static str,
    adapter: &AdapterId,
    payload: &BTreeMap<String, Value>,
) -> Result<(), PlanningError> {
    canonical_json_bytes(&json_object_value(payload))
        .map(|_| ())
        .map_err(|reason| PlanningError::InvalidPlannedPayload {
            kind,
            adapter: adapter.clone(),
            reason,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct StubObservationPlanner {
        adapter: AdapterId,
        match_family: NetworkFamily,
        match_semantics: super::super::InstrumentSemantics,
        emitted_binding: CompiledObservationBinding,
    }

    impl PlannerAdapter for StubObservationPlanner {
        fn id(&self) -> AdapterId {
            self.adapter.clone()
        }
    }

    impl ObservationPlannerAdapter for StubObservationPlanner {
        fn supports(&self, req: &ObservationPlanRequest<'_>) -> bool {
            req.network_view.family == self.match_family
                && req.instrument.semantics == self.match_semantics
        }

        fn plan(
            &self,
            _req: ObservationPlanRequest<'_>,
        ) -> Result<CompiledObservationBinding, PlanningError> {
            Ok(self.emitted_binding.clone())
        }
    }

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
            _binding: &CompiledObservationBinding,
            _input: ObservationRuntimeInput<'_>,
        ) -> Result<Observation, StateError> {
            unreachable!("runtime adapter is only used for lookup tests")
        }
    }

    fn sample_request<'a>(
        target: &'a ObservationTarget,
        subject: &'a Subject,
        view: &'a NetworkView,
        position: &'a Position,
        instrument: &'a super::super::Instrument,
        venue: Option<&'a super::super::Venue>,
        valuations: &'a [Valuation],
    ) -> ObservationPlanRequest<'a> {
        ObservationPlanRequest {
            target,
            subject,
            network_view: view,
            position,
            instrument,
            venue,
            valuations,
        }
    }

    fn sample_semantics() -> (
        ObservationTarget,
        Subject,
        NetworkView,
        Position,
        super::super::Instrument,
        super::super::Venue,
        Vec<Valuation>,
    ) {
        (
            ObservationTarget {
                target_id: "wallet_eth".to_string(),
                subject_id: "wallet_main".to_string(),
                network_view_id: "ethereum_live".to_string(),
                position_id: "eth_spot".to_string(),
                valuation_ids: vec!["eth_btc".to_string(), "eth_usd".to_string()],
                metadata: BTreeMap::new(),
            },
            Subject {
                subject_id: "wallet_main".to_string(),
                kind: SubjectKind::EvmAddress,
                locator: serde_json::json!({"address": "0x000000000000000000000000000000000000dead"}),
                metadata: BTreeMap::new(),
            },
            NetworkView {
                network_view_id: "ethereum_live".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                family: NetworkFamily::Evm,
                route_policy: serde_json::json!({"network_id": "ethereum-mainnet"}),
                metadata: BTreeMap::new(),
            },
            Position {
                position_id: "eth_spot".to_string(),
                instrument_id: "eth".to_string(),
                semantics: super::super::PositionSemantics::SpotBalance,
                venue_id: Some(super::super::VenueId("wallet".to_string())),
                reader_hint: None,
                metadata: BTreeMap::new(),
            },
            super::super::Instrument {
                instrument_id: "eth".to_string(),
                display_symbol: Some("ETH".to_string()),
                semantics: super::super::InstrumentSemantics::NativeAsset,
                quantity_schema: serde_json::json!({"decimals": 18}),
                metadata: BTreeMap::new(),
            },
            super::super::Venue {
                venue_id: super::super::VenueId("wallet".to_string()),
                display_name: Some("wallet".to_string()),
                kind: "wallet".to_string(),
                network_id: Some("ethereum-mainnet".to_string()),
                parent_venue_id: None,
                metadata: BTreeMap::new(),
            },
            vec![
                Valuation {
                    valuation_id: "eth_usd".to_string(),
                    instrument_id: "eth".to_string(),
                    quote: QuoteCode::Usd,
                    semantics: super::super::ValuationSemantics::DirectUnitPrice,
                    strategy: serde_json::json!({"source_id": "chainlink_eth_usd"}),
                    metadata: BTreeMap::new(),
                },
                Valuation {
                    valuation_id: "eth_btc".to_string(),
                    instrument_id: "eth".to_string(),
                    quote: QuoteCode::Btc,
                    semantics: super::super::ValuationSemantics::DirectUnitPrice,
                    strategy: serde_json::json!({"source_id": "chainlink_eth_btc"}),
                    metadata: BTreeMap::new(),
                },
            ],
        )
    }

    fn sample_binding(adapter: &str) -> CompiledObservationBinding {
        CompiledObservationBinding {
            binding_id: "binding.wallet_eth".to_string(),
            observation_key: super::super::ObservationKey {
                subject_id: "wallet_main".to_string(),
                network_view_id: "ethereum_live".to_string(),
                instrument_id: "eth".to_string(),
                position_kind: super::super::PositionSemantics::SpotBalance,
                venue_id: Some(super::super::VenueId("wallet".to_string())),
                discriminator: None,
            },
            adapter: AdapterId(adapter.to_string()),
            valuation_ids: vec!["eth_usd".to_string(), "eth_btc".to_string()],
            payload: BTreeMap::from([
                ("z".to_string(), serde_json::json!(1)),
                ("a".to_string(), serde_json::json!(2)),
            ]),
        }
    }

    #[test]
    fn selection_is_deterministic_across_registration_order() {
        let (target, subject, view, position, instrument, venue, valuations) = sample_semantics();
        let req = sample_request(
            &target,
            &subject,
            &view,
            &position,
            &instrument,
            Some(&venue),
            &valuations,
        );
        let matching = Arc::new(StubObservationPlanner {
            adapter: AdapterId("observe_position/evm/native_balance".to_string()),
            match_family: NetworkFamily::Evm,
            match_semantics: super::super::InstrumentSemantics::NativeAsset,
            emitted_binding: sample_binding("observe_position/evm/native_balance"),
        }) as Arc<dyn ObservationPlannerAdapter>;
        let non_matching = Arc::new(StubObservationPlanner {
            adapter: AdapterId("observe_position/bitcoin/utxo_set".to_string()),
            match_family: NetworkFamily::Bitcoin,
            match_semantics: super::super::InstrumentSemantics::NativeAsset,
            emitted_binding: sample_binding("observe_position/bitcoin/utxo_set"),
        }) as Arc<dyn ObservationPlannerAdapter>;

        let first = SemanticCatalog::new(SemanticCatalogParts {
            observation_planners: vec![Arc::clone(&matching), Arc::clone(&non_matching)],
            ..SemanticCatalogParts::default()
        })
        .expect("catalog")
        .plan_observation(req)
        .expect("planned observation");

        let (target, subject, view, position, instrument, venue, valuations) = sample_semantics();
        let second = SemanticCatalog::new(SemanticCatalogParts {
            observation_planners: vec![non_matching, matching],
            ..SemanticCatalogParts::default()
        })
        .expect("catalog")
        .plan_observation(sample_request(
            &target,
            &subject,
            &view,
            &position,
            &instrument,
            Some(&venue),
            &valuations,
        ))
        .expect("planned observation");

        assert_eq!(first, second);
    }

    #[test]
    fn selection_rejects_ambiguous_matches() {
        let (target, subject, view, position, instrument, venue, valuations) = sample_semantics();
        let first = Arc::new(StubObservationPlanner {
            adapter: AdapterId("observe_position/evm/a".to_string()),
            match_family: NetworkFamily::Evm,
            match_semantics: super::super::InstrumentSemantics::NativeAsset,
            emitted_binding: sample_binding("observe_position/evm/a"),
        }) as Arc<dyn ObservationPlannerAdapter>;
        let second = Arc::new(StubObservationPlanner {
            adapter: AdapterId("observe_position/evm/b".to_string()),
            match_family: NetworkFamily::Evm,
            match_semantics: super::super::InstrumentSemantics::NativeAsset,
            emitted_binding: sample_binding("observe_position/evm/b"),
        }) as Arc<dyn ObservationPlannerAdapter>;
        let catalog = SemanticCatalog::new(SemanticCatalogParts {
            observation_planners: vec![second, first],
            ..SemanticCatalogParts::default()
        })
        .expect("catalog");

        assert_eq!(
            catalog
                .plan_observation(sample_request(
                    &target,
                    &subject,
                    &view,
                    &position,
                    &instrument,
                    Some(&venue),
                    &valuations,
                ))
                .unwrap_err(),
            PlanningError::AmbiguousAdapterMatch {
                kind: "observation",
                adapters: vec![
                    AdapterId("observe_position/evm/a".to_string()),
                    AdapterId("observe_position/evm/b".to_string()),
                ],
            }
        );
    }

    #[test]
    fn selection_rejects_zero_matches() {
        let (target, subject, view, position, instrument, venue, valuations) = sample_semantics();
        let non_matching = Arc::new(StubObservationPlanner {
            adapter: AdapterId("observe_position/bitcoin/utxo_set".to_string()),
            match_family: NetworkFamily::Bitcoin,
            match_semantics: super::super::InstrumentSemantics::NativeAsset,
            emitted_binding: sample_binding("observe_position/bitcoin/utxo_set"),
        }) as Arc<dyn ObservationPlannerAdapter>;
        let catalog = SemanticCatalog::new(SemanticCatalogParts {
            observation_planners: vec![non_matching],
            ..SemanticCatalogParts::default()
        })
        .expect("catalog");

        assert_eq!(
            catalog
                .plan_observation(sample_request(
                    &target,
                    &subject,
                    &view,
                    &position,
                    &instrument,
                    Some(&venue),
                    &valuations,
                ))
                .unwrap_err(),
            PlanningError::NoMatchingAdapter {
                kind: "observation",
            }
        );
    }

    #[test]
    fn planning_normalizes_emitted_binding_payloads() {
        let (target, subject, view, position, instrument, venue, valuations) = sample_semantics();
        let planner = Arc::new(StubObservationPlanner {
            adapter: AdapterId("observe_position/evm/native_balance".to_string()),
            match_family: NetworkFamily::Evm,
            match_semantics: super::super::InstrumentSemantics::NativeAsset,
            emitted_binding: CompiledObservationBinding {
                valuation_ids: vec!["eth_usd".to_string(), "eth_btc".to_string()],
                ..sample_binding("observe_position/evm/native_balance")
            },
        }) as Arc<dyn ObservationPlannerAdapter>;
        let catalog = SemanticCatalog::new(SemanticCatalogParts {
            observation_planners: vec![planner],
            ..SemanticCatalogParts::default()
        })
        .expect("catalog");

        let binding = catalog
            .plan_observation(sample_request(
                &target,
                &subject,
                &view,
                &position,
                &instrument,
                Some(&venue),
                &valuations,
            ))
            .expect("planned observation");

        assert_eq!(
            binding.valuation_ids,
            vec!["eth_btc".to_string(), "eth_usd".to_string()]
        );
        assert_eq!(
            canonical_json_bytes(&json_object_value(&binding.payload)).unwrap(),
            br#"{"a":2,"z":1}"#
        );
    }

    #[test]
    fn runtime_lookup_uses_exact_planned_adapter_id() {
        let adapter = Arc::new(StubObservationRuntime {
            adapter: AdapterId("observe_position/evm/native_balance".to_string()),
        }) as Arc<dyn ObservationRuntimeAdapter>;
        let catalog = SemanticCatalog::new(SemanticCatalogParts {
            observation_runtimes: vec![Arc::clone(&adapter)],
            ..SemanticCatalogParts::default()
        })
        .expect("catalog");

        let resolved = catalog
            .observation_runtime(&AdapterId(
                "observe_position/evm/native_balance".to_string(),
            ))
            .expect("runtime lookup");
        assert_eq!(resolved.id(), adapter.id());
        let err = match catalog
            .observation_runtime(&AdapterId("observe_position/evm/other".to_string()))
        {
            Ok(_) => panic!("expected missing runtime adapter"),
            Err(err) => err,
        };
        assert_eq!(
            err,
            SemanticCatalogError::UnknownRuntimeAdapter {
                kind: "observation",
                adapter: AdapterId("observe_position/evm/other".to_string()),
            }
        );
    }
}
