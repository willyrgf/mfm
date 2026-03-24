use std::collections::{BTreeMap, BTreeSet};

use mfm_machine::hashing::{canonical_json_bytes, CanonicalJsonError};
use mfm_state_symbol::model::QuoteCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

mod catalog;
mod compiler;

pub use catalog::{
    ExecutionAnchor, ObservationPlanRequest, ObservationPlannerAdapter, ObservationRuntimeAdapter,
    ObservationRuntimeInput, PinnedNetworkView, PlannerAdapter, PlanningError, ResolvedSubject,
    ResolvedUnitPrice, RuntimeAdapter, SemanticCatalog, SemanticCatalogError, SemanticCatalogParts,
    SubjectPlanRequest, SubjectPlannerAdapter, SubjectRuntimeAdapter, SubjectRuntimeInput,
    ValuationPlanRequest, ValuationPlannerAdapter, ValuationRuntimeAdapter, ValuationRuntimeInput,
    ViewPlanRequest, ViewPlannerAdapter, ViewRuntimeAdapter, ViewRuntimeInput,
};
pub use compiler::{PortfolioRequest, PortfolioSemanticCompiler};
/// Canonical observation read model emitted by semantic portfolio execution.
///
/// The semantic cutover keeps the existing observation artifact shape so snapshot/report
/// projections remain stable while planner/runtime internals move to semantic compilation.
pub use mfm_state_symbol::model::Observation;

/// Stable semantic venue identifier authored by portfolio config.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VenueId(pub String);

impl std::fmt::Display for VenueId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable planner-selected adapter identifier.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AdapterId(pub String);

impl std::fmt::Display for AdapterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Supported semantic subject families.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    /// An EVM account or contract address subject.
    EvmAddress,
    /// A Bitcoin descriptor-backed subject.
    BitcoinDescriptor,
}

/// One semantic subject in the planner-owned portfolio model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    /// Stable subject identifier.
    pub subject_id: String,
    /// Semantic subject family.
    pub kind: SubjectKind,
    /// Opaque subject locator interpreted by planner/runtime adapters.
    pub locator: Value,
    /// Stable semantic metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl Subject {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {}
}

/// Supported execution-view families.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkFamily {
    /// EVM-family execution view.
    Evm,
    /// Bitcoin-family execution view.
    Bitcoin,
}

/// One planner-owned network view declaration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetworkView {
    /// Stable execution-view identifier.
    pub network_view_id: String,
    /// Stable user-authored network identifier.
    pub network_id: String,
    /// Semantic network family.
    pub family: NetworkFamily,
    /// Opaque planner-owned routing or source policy.
    pub route_policy: Value,
    /// Stable semantic metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl NetworkView {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {}
}

/// Supported instrument semantics.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentSemantics {
    /// Native ledger asset.
    NativeAsset,
    /// Fungible token or asset.
    FungibleToken,
    /// Quote unit used for valuation.
    QuoteUnit,
}

/// One semantic instrument declaration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instrument {
    /// Stable instrument identifier.
    pub instrument_id: String,
    /// Optional display symbol used in read models.
    pub display_symbol: Option<String>,
    /// Semantic instrument kind.
    pub semantics: InstrumentSemantics,
    /// Opaque quantity schema interpreted by planner/runtime adapters.
    pub quantity_schema: Value,
    /// Stable semantic metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl Instrument {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {}
}

/// Supported position semantics.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionSemantics {
    /// Spot account or wallet balance.
    SpotBalance,
    /// Lending-market collateral or deposit position.
    LendingDeposit,
    /// Lending-market debt position.
    LendingDebt,
    /// Liquidity-provider share position.
    LiquidityShare,
    /// Staked claim or receipt position.
    StakedClaim,
    /// UTXO-backed balance set.
    UtxoSet,
}

/// One semantic venue declaration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Venue {
    /// Stable venue identifier.
    pub venue_id: VenueId,
    /// Optional display name for audit/debug output.
    pub display_name: Option<String>,
    /// Semantic venue kind.
    pub kind: String,
    /// Optional network identifier associated with the venue.
    pub network_id: Option<String>,
    /// Optional parent venue for hierarchical semantic venues.
    pub parent_venue_id: Option<VenueId>,
    /// Stable semantic metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl Venue {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {}
}

/// One semantic position declaration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    /// Stable position identifier.
    pub position_id: String,
    /// Instrument tracked by this position.
    pub instrument_id: String,
    /// Semantic position kind.
    pub semantics: PositionSemantics,
    /// Optional semantic venue reference.
    pub venue_id: Option<VenueId>,
    /// Optional planner-side reader hint.
    pub reader_hint: Option<String>,
    /// Stable semantic metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl Position {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {}
}

/// Supported valuation semantics.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValuationSemantics {
    /// Fixed unit price encoded in planner config.
    FixedUnitPrice,
    /// Direct unit price read from an execution source.
    DirectUnitPrice,
    /// Derived unit price composed from other valuation inputs.
    DerivedUnitPrice,
}

/// One semantic valuation declaration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Valuation {
    /// Stable valuation identifier.
    pub valuation_id: String,
    /// Instrument valued by this task.
    pub instrument_id: String,
    /// Quote unit emitted by the valuation.
    pub quote: QuoteCode,
    /// Semantic valuation strategy kind.
    pub semantics: ValuationSemantics,
    /// Opaque adapter-owned valuation strategy payload.
    pub strategy: Value,
    /// Stable semantic metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl Valuation {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {}
}

/// One semantic observation target linking subject, view, position, and valuations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservationTarget {
    /// Stable target identifier.
    pub target_id: String,
    /// Subject observed by this target.
    pub subject_id: String,
    /// Pinned execution view used by this target.
    pub network_view_id: String,
    /// Position observed by this target.
    pub position_id: String,
    /// Valuations attached to observations emitted by this target.
    pub valuation_ids: Vec<String>,
    /// Stable semantic metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl ObservationTarget {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {
        self.valuation_ids.sort();
    }
}

/// Planner-owned semantic portfolio configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortfolioSemanticConfig {
    /// Stable portfolio identifier.
    pub portfolio_id: String,
    /// Quote units requested by the portfolio.
    pub quote_codes: Vec<QuoteCode>,
    /// Available execution views.
    pub network_views: Vec<NetworkView>,
    /// Declared subjects.
    pub subjects: Vec<Subject>,
    /// Declared instruments.
    pub instruments: Vec<Instrument>,
    /// Declared venues.
    pub venues: Vec<Venue>,
    /// Declared positions.
    pub positions: Vec<Position>,
    /// Declared valuations.
    pub valuations: Vec<Valuation>,
    /// Declared observation targets.
    pub observation_targets: Vec<ObservationTarget>,
    /// Stable semantic metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl PortfolioSemanticConfig {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {
        self.quote_codes.sort();
        self.network_views
            .sort_by(|left, right| left.network_view_id.cmp(&right.network_view_id));
        for view in &mut self.network_views {
            view.normalize();
        }
        self.subjects
            .sort_by(|left, right| left.subject_id.cmp(&right.subject_id));
        for subject in &mut self.subjects {
            subject.normalize();
        }
        self.instruments
            .sort_by(|left, right| left.instrument_id.cmp(&right.instrument_id));
        for instrument in &mut self.instruments {
            instrument.normalize();
        }
        self.venues
            .sort_by(|left, right| left.venue_id.cmp(&right.venue_id));
        for venue in &mut self.venues {
            venue.normalize();
        }
        self.positions
            .sort_by(|left, right| left.position_id.cmp(&right.position_id));
        for position in &mut self.positions {
            position.normalize();
        }
        self.valuations
            .sort_by(|left, right| left.valuation_id.cmp(&right.valuation_id));
        for valuation in &mut self.valuations {
            valuation.normalize();
        }
        self.observation_targets
            .sort_by(|left, right| left.target_id.cmp(&right.target_id));
        for target in &mut self.observation_targets {
            target.normalize();
        }
    }

    /// Returns a normalized clone of the semantic config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    /// Validates semantic identities, references, and canonical-json-safe opaque fields.
    pub fn validate(&self) -> Result<(), SemanticConfigError> {
        ensure_non_empty_id("portfolio", &self.portfolio_id)?;
        ensure_canonical_json_object("portfolio", &self.portfolio_id, "metadata", &self.metadata)?;

        let mut seen_quotes = BTreeSet::new();
        for quote in &self.quote_codes {
            if !seen_quotes.insert(*quote) {
                return Err(SemanticConfigError::DuplicateQuoteCode { quote: *quote });
            }
        }

        let network_view_ids = unique_ids(
            "network_view",
            self.network_views.iter().map(|view| &view.network_view_id),
        )?;
        for view in &self.network_views {
            ensure_canonical_json_value(
                "network_view",
                &view.network_view_id,
                "route_policy",
                &view.route_policy,
            )?;
            ensure_canonical_json_object(
                "network_view",
                &view.network_view_id,
                "metadata",
                &view.metadata,
            )?;
        }

        let subject_ids = unique_ids(
            "subject",
            self.subjects.iter().map(|subject| &subject.subject_id),
        )?;
        for subject in &self.subjects {
            ensure_canonical_json_value(
                "subject",
                &subject.subject_id,
                "locator",
                &subject.locator,
            )?;
            ensure_canonical_json_object(
                "subject",
                &subject.subject_id,
                "metadata",
                &subject.metadata,
            )?;
        }

        let instrument_ids = unique_ids(
            "instrument",
            self.instruments
                .iter()
                .map(|instrument| &instrument.instrument_id),
        )?;
        for instrument in &self.instruments {
            ensure_canonical_json_value(
                "instrument",
                &instrument.instrument_id,
                "quantity_schema",
                &instrument.quantity_schema,
            )?;
            ensure_canonical_json_object(
                "instrument",
                &instrument.instrument_id,
                "metadata",
                &instrument.metadata,
            )?;
        }

        let venue_ids = unique_ids("venue", self.venues.iter().map(|venue| &venue.venue_id.0))?;
        for venue in &self.venues {
            ensure_canonical_json_object("venue", &venue.venue_id.0, "metadata", &venue.metadata)?;
        }

        let position_ids = unique_ids(
            "position",
            self.positions.iter().map(|position| &position.position_id),
        )?;
        for position in &self.positions {
            if !instrument_ids.contains(position.instrument_id.as_str()) {
                return Err(SemanticConfigError::UnknownReference {
                    entity: "position",
                    id: position.position_id.clone(),
                    field: "instrument_id",
                    target: position.instrument_id.clone(),
                });
            }
            if let Some(venue_id) = &position.venue_id {
                if !venue_ids.contains(venue_id.0.as_str()) {
                    return Err(SemanticConfigError::UnknownReference {
                        entity: "position",
                        id: position.position_id.clone(),
                        field: "venue_id",
                        target: venue_id.0.clone(),
                    });
                }
            }
            ensure_canonical_json_object(
                "position",
                &position.position_id,
                "metadata",
                &position.metadata,
            )?;
        }

        let valuation_ids = unique_ids(
            "valuation",
            self.valuations
                .iter()
                .map(|valuation| &valuation.valuation_id),
        )?;
        for valuation in &self.valuations {
            if !instrument_ids.contains(valuation.instrument_id.as_str()) {
                return Err(SemanticConfigError::UnknownReference {
                    entity: "valuation",
                    id: valuation.valuation_id.clone(),
                    field: "instrument_id",
                    target: valuation.instrument_id.clone(),
                });
            }
            ensure_canonical_json_value(
                "valuation",
                &valuation.valuation_id,
                "strategy",
                &valuation.strategy,
            )?;
            ensure_canonical_json_object(
                "valuation",
                &valuation.valuation_id,
                "metadata",
                &valuation.metadata,
            )?;
        }

        let mut seen_targets = BTreeSet::new();
        for target in &self.observation_targets {
            if !seen_targets.insert(target.target_id.as_str()) {
                return Err(SemanticConfigError::DuplicateId {
                    entity: "observation_target",
                    id: target.target_id.clone(),
                });
            }
            if !subject_ids.contains(target.subject_id.as_str()) {
                return Err(SemanticConfigError::UnknownReference {
                    entity: "observation_target",
                    id: target.target_id.clone(),
                    field: "subject_id",
                    target: target.subject_id.clone(),
                });
            }
            if !network_view_ids.contains(target.network_view_id.as_str()) {
                return Err(SemanticConfigError::UnknownReference {
                    entity: "observation_target",
                    id: target.target_id.clone(),
                    field: "network_view_id",
                    target: target.network_view_id.clone(),
                });
            }
            if !position_ids.contains(target.position_id.as_str()) {
                return Err(SemanticConfigError::UnknownReference {
                    entity: "observation_target",
                    id: target.target_id.clone(),
                    field: "position_id",
                    target: target.position_id.clone(),
                });
            }
            let mut seen_target_valuations = BTreeSet::new();
            for valuation_id in &target.valuation_ids {
                if !seen_target_valuations.insert(valuation_id.as_str()) {
                    return Err(SemanticConfigError::DuplicateId {
                        entity: "observation_target.valuation_id",
                        id: valuation_id.clone(),
                    });
                }
                if !valuation_ids.contains(valuation_id.as_str()) {
                    return Err(SemanticConfigError::UnknownReference {
                        entity: "observation_target",
                        id: target.target_id.clone(),
                        field: "valuation_ids",
                        target: valuation_id.clone(),
                    });
                }
            }
            ensure_canonical_json_object(
                "observation_target",
                &target.target_id,
                "metadata",
                &target.metadata,
            )?;
        }

        Ok(())
    }
}

/// Explicit planner-owned semantic identity for one observation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObservationKey {
    /// Stable semantic subject identifier.
    pub subject_id: String,
    /// Stable semantic execution-view identifier.
    pub network_view_id: String,
    /// Stable semantic instrument identifier.
    pub instrument_id: String,
    /// Stable semantic position kind.
    pub position_kind: PositionSemantics,
    /// Optional semantic venue identifier.
    pub venue_id: Option<VenueId>,
    /// Optional planner-owned discriminator for multiple rows in one semantic bucket.
    pub discriminator: Option<String>,
}

/// Planner-owned source preparation task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourcePreparationTask {
    /// Stable preparation task identifier.
    pub task_id: String,
    /// Semantic network view prepared by this task.
    pub network_view_id: String,
    /// Network family prepared by this task.
    pub family: NetworkFamily,
    /// Adapter-owned canonical payload.
    pub payload: BTreeMap<String, Value>,
}

/// Planner-owned subject resolution task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubjectResolutionTask {
    /// Stable subject task identifier.
    pub task_id: String,
    /// Semantic subject resolved by this task.
    pub subject_id: String,
    /// Subject family resolved by this task.
    pub kind: SubjectKind,
    /// Runtime adapter identifier selected by the planner.
    pub adapter: AdapterId,
    /// Adapter-owned canonical payload.
    pub payload: BTreeMap<String, Value>,
}

/// Planner-owned execution-view pin task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewPinTask {
    /// Stable view pin task identifier.
    pub task_id: String,
    /// Semantic network view pinned by this task.
    pub network_view_id: String,
    /// Network family pinned by this task.
    pub family: NetworkFamily,
    /// Runtime adapter identifier selected by the planner.
    pub adapter: AdapterId,
    /// Adapter-owned canonical payload.
    pub payload: BTreeMap<String, Value>,
}

/// Planner-owned valuation task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValuationTask {
    /// Stable valuation task identifier. This doubles as the semantic valuation id in v1.
    pub valuation_id: String,
    /// Instrument valued by this task.
    pub instrument_id: String,
    /// Quote unit emitted by this task.
    pub quote: QuoteCode,
    /// Runtime adapter identifier selected by the planner.
    pub adapter: AdapterId,
    /// Adapter-owned canonical payload.
    pub payload: BTreeMap<String, Value>,
}

/// One compiled observation binding emitted by the semantic compiler.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledObservationBinding {
    /// Stable binding identifier.
    pub binding_id: String,
    /// Explicit semantic observation identity.
    pub observation_key: ObservationKey,
    /// Runtime adapter identifier selected by the planner.
    pub adapter: AdapterId,
    /// Valuation tasks required for this observation.
    pub valuation_ids: Vec<String>,
    /// Adapter-owned canonical payload.
    pub payload: BTreeMap<String, Value>,
}

impl CompiledObservationBinding {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {
        self.valuation_ids.sort();
    }
}

/// One compiled homogeneous observation batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledObservationBatch {
    /// Stable batch identifier.
    pub batch_id: String,
    /// Runtime adapter identifier shared by every binding in the batch.
    pub adapter: AdapterId,
    /// Semantic execution view shared by every binding in the batch.
    pub network_view_id: String,
    /// Concrete observation bindings in deterministic planner order.
    pub bindings: Vec<CompiledObservationBinding>,
}

impl CompiledObservationBatch {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {
        self.bindings.sort_by(|left, right| {
            left.observation_key
                .cmp(&right.observation_key)
                .then_with(|| left.binding_id.cmp(&right.binding_id))
        });
        for binding in &mut self.bindings {
            binding.normalize();
        }
    }
}

/// Planner-owned compiled execution spec for semantic portfolio execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortfolioExecutionSpec {
    /// Stable portfolio identifier.
    pub portfolio_id: String,
    /// Quote units requested by the portfolio.
    pub quote_codes: Vec<QuoteCode>,
    /// Planner-owned source preparation tasks.
    pub source_tasks: Vec<SourcePreparationTask>,
    /// Planner-owned subject resolution tasks.
    pub subject_tasks: Vec<SubjectResolutionTask>,
    /// Planner-owned execution-view pin tasks.
    pub view_tasks: Vec<ViewPinTask>,
    /// Planner-owned valuation tasks.
    pub valuation_tasks: Vec<ValuationTask>,
    /// Planner-owned compiled observation batches.
    pub observation_batches: Vec<CompiledObservationBatch>,
}

impl PortfolioExecutionSpec {
    /// Sorts nested collections into a deterministic canonical order.
    pub fn normalize(&mut self) {
        self.quote_codes.sort();
        self.source_tasks
            .sort_by(|left, right| left.task_id.cmp(&right.task_id));
        self.subject_tasks
            .sort_by(|left, right| left.task_id.cmp(&right.task_id));
        self.view_tasks
            .sort_by(|left, right| left.task_id.cmp(&right.task_id));
        self.valuation_tasks
            .sort_by(|left, right| left.valuation_id.cmp(&right.valuation_id));
        self.observation_batches
            .sort_by(|left, right| left.batch_id.cmp(&right.batch_id));
        for batch in &mut self.observation_batches {
            batch.normalize();
        }
    }

    /// Returns a normalized clone of the compiled execution spec.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    /// Validates planner-owned execution identities, batch homogeneity, and payload safety.
    pub fn validate(&self) -> Result<(), SemanticExecutionSpecError> {
        ensure_non_empty_execution_id("portfolio", &self.portfolio_id)?;

        let mut seen_quotes = BTreeSet::new();
        for quote in &self.quote_codes {
            if !seen_quotes.insert(*quote) {
                return Err(SemanticExecutionSpecError::DuplicateQuoteCode { quote: *quote });
            }
        }

        unique_execution_ids(
            "source_task",
            self.source_tasks.iter().map(|task| &task.task_id),
        )?;
        for task in &self.source_tasks {
            ensure_non_empty_execution_id("network_view_id", &task.network_view_id)?;
            ensure_canonical_execution_payload("source_task", &task.task_id, &task.payload)?;
        }

        unique_execution_ids(
            "subject_task",
            self.subject_tasks.iter().map(|task| &task.task_id),
        )?;
        for task in &self.subject_tasks {
            ensure_non_empty_execution_id("subject_id", &task.subject_id)?;
            ensure_non_empty_execution_id("adapter", &task.adapter.0)?;
            ensure_canonical_execution_payload("subject_task", &task.task_id, &task.payload)?;
        }

        unique_execution_ids(
            "view_task",
            self.view_tasks.iter().map(|task| &task.task_id),
        )?;
        for task in &self.view_tasks {
            ensure_non_empty_execution_id("network_view_id", &task.network_view_id)?;
            ensure_non_empty_execution_id("adapter", &task.adapter.0)?;
            ensure_canonical_execution_payload("view_task", &task.task_id, &task.payload)?;
        }

        let valuation_ids = unique_execution_ids(
            "valuation_task",
            self.valuation_tasks.iter().map(|task| &task.valuation_id),
        )?;
        for task in &self.valuation_tasks {
            ensure_non_empty_execution_id("instrument_id", &task.instrument_id)?;
            ensure_non_empty_execution_id("adapter", &task.adapter.0)?;
            ensure_canonical_execution_payload(
                "valuation_task",
                &task.valuation_id,
                &task.payload,
            )?;
        }

        let batch_ids = unique_execution_ids(
            "observation_batch",
            self.observation_batches.iter().map(|batch| &batch.batch_id),
        )?;
        let mut seen_observation_keys = BTreeSet::new();
        for batch in &self.observation_batches {
            let _ = &batch_ids;
            ensure_non_empty_execution_id("adapter", &batch.adapter.0)?;
            ensure_non_empty_execution_id("network_view_id", &batch.network_view_id)?;
            let _ = unique_execution_ids(
                "compiled_observation_binding",
                batch.bindings.iter().map(|binding| &binding.binding_id),
            )?;
            for binding in &batch.bindings {
                ensure_non_empty_execution_id("adapter", &binding.adapter.0)?;
                if binding.adapter != batch.adapter {
                    return Err(SemanticExecutionSpecError::BatchAdapterMismatch {
                        batch_id: batch.batch_id.clone(),
                        binding_id: binding.binding_id.clone(),
                        batch_adapter: batch.adapter.clone(),
                        binding_adapter: binding.adapter.clone(),
                    });
                }
                if binding.observation_key.network_view_id != batch.network_view_id {
                    return Err(SemanticExecutionSpecError::BatchNetworkViewMismatch {
                        batch_id: batch.batch_id.clone(),
                        binding_id: binding.binding_id.clone(),
                        batch_network_view_id: batch.network_view_id.clone(),
                        binding_network_view_id: binding.observation_key.network_view_id.clone(),
                    });
                }
                if !seen_observation_keys.insert(binding.observation_key.clone()) {
                    return Err(SemanticExecutionSpecError::DuplicateObservationKey {
                        key: Box::new(binding.observation_key.clone()),
                    });
                }
                let mut seen_binding_valuations = BTreeSet::new();
                for valuation_id in &binding.valuation_ids {
                    if !seen_binding_valuations.insert(valuation_id.as_str()) {
                        return Err(SemanticExecutionSpecError::DuplicateId {
                            kind: "compiled_observation_binding.valuation_id",
                            id: valuation_id.clone(),
                        });
                    }
                    if !valuation_ids.contains(valuation_id.as_str()) {
                        return Err(SemanticExecutionSpecError::UnknownValuationId {
                            binding_id: binding.binding_id.clone(),
                            valuation_id: valuation_id.clone(),
                        });
                    }
                }
                ensure_canonical_execution_payload(
                    "compiled_observation_binding",
                    &binding.binding_id,
                    &binding.payload,
                )?;
            }
        }

        Ok(())
    }
}

/// Validation errors for semantic portfolio config.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SemanticConfigError {
    /// One semantic identifier was empty.
    #[error("{entity} id must be non-empty")]
    EmptyId {
        /// Semantic entity owning the identifier.
        entity: &'static str,
    },
    /// One semantic identifier was declared more than once.
    #[error("duplicate {entity} id `{id}`")]
    DuplicateId {
        /// Semantic entity owning the identifier.
        entity: &'static str,
        /// Duplicate identifier value.
        id: String,
    },
    /// One semantic reference pointed at an unknown identifier.
    #[error("{entity} `{id}` referenced unknown {field} `{target}`")]
    UnknownReference {
        /// Semantic entity containing the reference.
        entity: &'static str,
        /// Identifier of the entity containing the reference.
        id: String,
        /// Field that contained the bad reference.
        field: &'static str,
        /// Referenced identifier that was not found.
        target: String,
    },
    /// One opaque semantic payload violated canonical JSON rules.
    #[error("{entity} `{id}` contained non-canonical JSON in `{field}`: {reason}")]
    InvalidCanonicalJson {
        /// Semantic entity containing the payload.
        entity: &'static str,
        /// Identifier of the entity containing the payload.
        id: String,
        /// Payload field that failed validation.
        field: &'static str,
        /// Underlying canonical-json failure.
        reason: CanonicalJsonError,
    },
    /// Quote codes must be unique at the semantic root.
    #[error("duplicate quote code `{quote}`")]
    DuplicateQuoteCode {
        /// Duplicated quote code.
        quote: QuoteCode,
    },
}

/// Validation errors for compiled semantic execution specs.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SemanticExecutionSpecError {
    /// One compiled execution identifier was empty.
    #[error("{kind} must be non-empty")]
    EmptyId {
        /// Kind of identifier that was empty.
        kind: &'static str,
    },
    /// One compiled execution identifier was duplicated.
    #[error("duplicate {kind} `{id}`")]
    DuplicateId {
        /// Kind of identifier that was duplicated.
        kind: &'static str,
        /// Duplicate identifier value.
        id: String,
    },
    /// Quote codes must be unique at execution time too.
    #[error("duplicate quote code `{quote}`")]
    DuplicateQuoteCode {
        /// Duplicated quote code.
        quote: QuoteCode,
    },
    /// Two compiled bindings attempted to claim the same semantic observation identity.
    #[error("duplicate observation key `{key:?}`")]
    DuplicateObservationKey {
        /// Duplicate semantic observation identity.
        key: Box<ObservationKey>,
    },
    /// A binding referenced a valuation task that does not exist in the execution spec.
    #[error("binding `{binding_id}` referenced unknown valuation `{valuation_id}`")]
    UnknownValuationId {
        /// Binding that referenced the unknown valuation.
        binding_id: String,
        /// Missing valuation identifier.
        valuation_id: String,
    },
    /// A compiled batch mixed adapters, which violates the homogeneous batch contract.
    #[error(
        "observation batch `{batch_id}` mixed binding `{binding_id}` adapter `{binding_adapter}` with batch adapter `{batch_adapter}`"
    )]
    BatchAdapterMismatch {
        /// Batch that contained the mismatch.
        batch_id: String,
        /// Binding that violated the batch contract.
        binding_id: String,
        /// Adapter selected for the batch.
        batch_adapter: AdapterId,
        /// Adapter carried by the binding.
        binding_adapter: AdapterId,
    },
    /// A compiled batch mixed execution views, which violates the homogeneous batch contract.
    #[error(
        "observation batch `{batch_id}` mixed binding `{binding_id}` network_view_id `{binding_network_view_id}` with batch network_view_id `{batch_network_view_id}`"
    )]
    BatchNetworkViewMismatch {
        /// Batch that contained the mismatch.
        batch_id: String,
        /// Binding that violated the batch contract.
        binding_id: String,
        /// Network view selected for the batch.
        batch_network_view_id: String,
        /// Network view carried by the binding.
        binding_network_view_id: String,
    },
    /// One adapter-owned compiled payload violated canonical JSON rules.
    #[error("{entity} `{id}` contained non-canonical payload JSON: {reason}")]
    InvalidCanonicalPayload {
        /// Execution entity containing the payload.
        entity: &'static str,
        /// Identifier of the execution entity containing the payload.
        id: String,
        /// Underlying canonical-json failure.
        reason: CanonicalJsonError,
    },
}

fn json_object_value(map: &BTreeMap<String, Value>) -> Value {
    let mut object = Map::new();
    for (key, value) in map {
        object.insert(key.clone(), value.clone());
    }
    Value::Object(object)
}

fn ensure_non_empty_id(entity: &'static str, id: &str) -> Result<(), SemanticConfigError> {
    if id.is_empty() {
        return Err(SemanticConfigError::EmptyId { entity });
    }
    Ok(())
}

fn ensure_non_empty_execution_id(
    kind: &'static str,
    id: &str,
) -> Result<(), SemanticExecutionSpecError> {
    if id.is_empty() {
        return Err(SemanticExecutionSpecError::EmptyId { kind });
    }
    Ok(())
}

fn unique_ids<'a>(
    entity: &'static str,
    ids: impl Iterator<Item = &'a String>,
) -> Result<BTreeSet<&'a str>, SemanticConfigError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        ensure_non_empty_id(entity, id)?;
        if !seen.insert(id.as_str()) {
            return Err(SemanticConfigError::DuplicateId {
                entity,
                id: id.clone(),
            });
        }
    }
    Ok(seen)
}

fn unique_execution_ids<'a>(
    kind: &'static str,
    ids: impl Iterator<Item = &'a String>,
) -> Result<BTreeSet<&'a str>, SemanticExecutionSpecError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        ensure_non_empty_execution_id(kind, id)?;
        if !seen.insert(id.as_str()) {
            return Err(SemanticExecutionSpecError::DuplicateId {
                kind,
                id: id.clone(),
            });
        }
    }
    Ok(seen)
}

fn ensure_canonical_json_value(
    entity: &'static str,
    id: &str,
    field: &'static str,
    value: &Value,
) -> Result<(), SemanticConfigError> {
    canonical_json_bytes(value).map(|_| ()).map_err(|reason| {
        SemanticConfigError::InvalidCanonicalJson {
            entity,
            id: id.to_string(),
            field,
            reason,
        }
    })
}

fn ensure_canonical_json_object(
    entity: &'static str,
    id: &str,
    field: &'static str,
    value: &BTreeMap<String, Value>,
) -> Result<(), SemanticConfigError> {
    ensure_canonical_json_value(entity, id, field, &json_object_value(value))
}

fn ensure_canonical_execution_payload(
    entity: &'static str,
    id: &str,
    payload: &BTreeMap<String, Value>,
) -> Result<(), SemanticExecutionSpecError> {
    canonical_json_bytes(&json_object_value(payload))
        .map(|_| ())
        .map_err(
            |reason| SemanticExecutionSpecError::InvalidCanonicalPayload {
                entity,
                id: id.to_string(),
                reason,
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_semantic_config() -> PortfolioSemanticConfig {
        PortfolioSemanticConfig {
            portfolio_id: "portfolio_main".to_string(),
            quote_codes: vec![QuoteCode::Usd],
            network_views: vec![NetworkView {
                network_view_id: "ethereum_live".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                family: NetworkFamily::Evm,
                route_policy: serde_json::json!({"network_id": "ethereum-mainnet"}),
                metadata: BTreeMap::new(),
            }],
            subjects: vec![Subject {
                subject_id: "wallet_main".to_string(),
                kind: SubjectKind::EvmAddress,
                locator: serde_json::json!({"address": "0x000000000000000000000000000000000000dead"}),
                metadata: BTreeMap::new(),
            }],
            instruments: vec![Instrument {
                instrument_id: "eth".to_string(),
                display_symbol: Some("ETH".to_string()),
                semantics: InstrumentSemantics::NativeAsset,
                quantity_schema: serde_json::json!({"decimals": 18}),
                metadata: BTreeMap::new(),
            }],
            venues: vec![Venue {
                venue_id: VenueId("wallet".to_string()),
                display_name: Some("wallet".to_string()),
                kind: "wallet".to_string(),
                network_id: Some("ethereum-mainnet".to_string()),
                parent_venue_id: None,
                metadata: BTreeMap::new(),
            }],
            positions: vec![Position {
                position_id: "eth_spot".to_string(),
                instrument_id: "eth".to_string(),
                semantics: PositionSemantics::SpotBalance,
                venue_id: Some(VenueId("wallet".to_string())),
                reader_hint: None,
                metadata: BTreeMap::new(),
            }],
            valuations: vec![Valuation {
                valuation_id: "eth_usd".to_string(),
                instrument_id: "eth".to_string(),
                quote: QuoteCode::Usd,
                semantics: ValuationSemantics::DirectUnitPrice,
                strategy: serde_json::json!({"source_id": "chainlink_eth_usd"}),
                metadata: BTreeMap::new(),
            }],
            observation_targets: vec![ObservationTarget {
                target_id: "wallet_eth".to_string(),
                subject_id: "wallet_main".to_string(),
                network_view_id: "ethereum_live".to_string(),
                position_id: "eth_spot".to_string(),
                valuation_ids: vec!["eth_usd".to_string()],
                metadata: BTreeMap::new(),
            }],
            metadata: BTreeMap::new(),
        }
    }

    fn sample_execution_spec() -> PortfolioExecutionSpec {
        PortfolioExecutionSpec {
            portfolio_id: "portfolio_main".to_string(),
            quote_codes: vec![QuoteCode::Usd],
            source_tasks: vec![SourcePreparationTask {
                task_id: "prepare.ethereum_live".to_string(),
                network_view_id: "ethereum_live".to_string(),
                family: NetworkFamily::Evm,
                payload: BTreeMap::from([(
                    "network_id".to_string(),
                    serde_json::json!("ethereum-mainnet"),
                )]),
            }],
            subject_tasks: vec![SubjectResolutionTask {
                task_id: "resolve.wallet_main".to_string(),
                subject_id: "wallet_main".to_string(),
                kind: SubjectKind::EvmAddress,
                adapter: AdapterId("resolve_subject/evm_address".to_string()),
                payload: BTreeMap::from([(
                    "address".to_string(),
                    serde_json::json!("0x000000000000000000000000000000000000dead"),
                )]),
            }],
            view_tasks: vec![ViewPinTask {
                task_id: "pin.ethereum_live".to_string(),
                network_view_id: "ethereum_live".to_string(),
                family: NetworkFamily::Evm,
                adapter: AdapterId("pin_view/evm".to_string()),
                payload: BTreeMap::from([(
                    "network_id".to_string(),
                    serde_json::json!("ethereum-mainnet"),
                )]),
            }],
            valuation_tasks: vec![ValuationTask {
                valuation_id: "eth_usd".to_string(),
                instrument_id: "eth".to_string(),
                quote: QuoteCode::Usd,
                adapter: AdapterId("resolve_valuation/direct_price".to_string()),
                payload: BTreeMap::from([(
                    "source_id".to_string(),
                    serde_json::json!("chainlink_eth_usd"),
                )]),
            }],
            observation_batches: vec![CompiledObservationBatch {
                batch_id: "observe.ethereum_live.spot".to_string(),
                adapter: AdapterId("observe_position/evm/native_balance".to_string()),
                network_view_id: "ethereum_live".to_string(),
                bindings: vec![CompiledObservationBinding {
                    binding_id: "binding.wallet_eth".to_string(),
                    observation_key: ObservationKey {
                        subject_id: "wallet_main".to_string(),
                        network_view_id: "ethereum_live".to_string(),
                        instrument_id: "eth".to_string(),
                        position_kind: PositionSemantics::SpotBalance,
                        venue_id: Some(VenueId("wallet".to_string())),
                        discriminator: None,
                    },
                    adapter: AdapterId("observe_position/evm/native_balance".to_string()),
                    valuation_ids: vec!["eth_usd".to_string()],
                    payload: BTreeMap::from([(
                        "subject_id".to_string(),
                        serde_json::json!("wallet_main"),
                    )]),
                }],
            }],
        }
    }

    #[test]
    fn semantic_config_validates_and_normalizes() {
        let mut cfg = sample_semantic_config();
        cfg.quote_codes.push(QuoteCode::Btc);
        cfg.quote_codes.reverse();
        cfg.normalize();

        assert_eq!(cfg.quote_codes, vec![QuoteCode::Btc, QuoteCode::Usd]);
        cfg.validate().expect("semantic config");
    }

    #[test]
    fn semantic_config_rejects_duplicate_venue_ids() {
        let mut cfg = sample_semantic_config();
        cfg.venues.push(Venue {
            venue_id: VenueId("wallet".to_string()),
            display_name: None,
            kind: "wallet".to_string(),
            network_id: Some("ethereum-mainnet".to_string()),
            parent_venue_id: None,
            metadata: BTreeMap::new(),
        });

        assert_eq!(
            cfg.validate().unwrap_err(),
            SemanticConfigError::DuplicateId {
                entity: "venue",
                id: "wallet".to_string(),
            }
        );
    }

    #[test]
    fn semantic_config_rejects_unknown_target_reference() {
        let mut cfg = sample_semantic_config();
        cfg.observation_targets[0].position_id = "missing".to_string();

        assert_eq!(
            cfg.validate().unwrap_err(),
            SemanticConfigError::UnknownReference {
                entity: "observation_target",
                id: "wallet_eth".to_string(),
                field: "position_id",
                target: "missing".to_string(),
            }
        );
    }

    #[test]
    fn execution_spec_rejects_duplicate_observation_keys() {
        let mut spec = sample_execution_spec();
        let mut duplicate = spec.observation_batches[0].bindings[0].clone();
        duplicate.binding_id = "binding.wallet_eth.duplicate".to_string();
        spec.observation_batches[0].bindings.push(duplicate);

        assert!(matches!(
            spec.validate().unwrap_err(),
            SemanticExecutionSpecError::DuplicateObservationKey { .. }
        ));
    }

    #[test]
    fn execution_spec_rejects_non_canonical_payloads() {
        let mut spec = sample_execution_spec();
        spec.valuation_tasks[0]
            .payload
            .insert("x".to_string(), serde_json::json!(1.5));

        assert_eq!(
            spec.validate().unwrap_err(),
            SemanticExecutionSpecError::InvalidCanonicalPayload {
                entity: "valuation_task",
                id: "eth_usd".to_string(),
                reason: CanonicalJsonError::FloatNotAllowed,
            }
        );
    }
}
