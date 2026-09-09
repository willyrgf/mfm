#![warn(missing_docs)]
//! Secret-free sequential Portfolio State values.
//!
//! Portfolio owns one singular admission context and one cumulative continuation.  Each EVM
//! collection is expanded into an ordinary sequential child State; there is no runtime collection
//! loop, output map, parallel branch, or multi-result join.

mod bounds;
pub use enrichment::{
    plan_enrichment, EnrichmentProvenance, PortfolioAdmission, PortfolioEnrichmentOutput,
    ResolvePortfolioAssets, PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID,
};

use std::collections::BTreeSet;
use std::num::NonZeroU64;

use mfm_evm::{
    CollectEvmBalances, EvmBalanceCollectionCompletion, EvmBalanceContext, EvmBalanceFailure,
    EvmBalanceRequest, EvmBalanceSource, EvmPhysicalTarget,
};
use mfm_ids::{ContentRef, EntryPointId, StableId};
use mfm_program::{
    expand_program, Operation, OperationExpansion, Program, ProposedStateOutcome, PureState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::string_contains_secret_marker;
use serde::de;
use serde::{Deserialize, Serialize};

macro_rules! impl_checked_deserialize {
    ($type:ident { $($field:ident: $field_type:ty),+ $(,)? }) => {
        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Wire {
                    $($field: $field_type,)+
                }

                let wire = Wire::deserialize(deserializer)?;
                let value = Self {
                    $($field: wire.$field,)+
                };
                value.validate().map(|_| value).map_err(de::Error::custom)
            }
        }
    };
}

/// Stable Portfolio snapshot entry-point identity.
pub const PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID: &str = "mfm.portfolio/snapshot@1";
/// Human-readable Portfolio snapshot entry-point description.
pub const PORTFOLIO_SNAPSHOT_ENTRY_POINT_DESCRIPTION: &str =
    "Builds a Portfolio snapshot from configured EVM observations.";
/// Maximum declaration-ordered EVM collections.
const PORTFOLIO_COLLECTION_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum QuoteCode {
    Usd,
    Eur,
}

/// One public Portfolio identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(transparent)]
pub struct PortfolioId {
    /// Stable public spelling.
    pub value: String,
}

impl<'de> Deserialize<'de> for PortfolioId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if !valid_public_text(&value, 256) {
            return Err(de::Error::custom(PortfolioError::InvalidValue));
        }
        Ok(Self { value })
    }
}

/// One exact collection demand selected by the trusted Portfolio planner.
///
/// The route identity is secret-free evidence of the binding selected during planning. It is not
/// a provider handle and cannot be supplied by a transport selector.
#[derive(Debug, Clone, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioCollectionDemand {
    correlation: String,
    request: EvmBalanceRequest,
    route_ref: ContentRef,
}

impl_checked_deserialize!(PortfolioCollectionDemand {
    correlation: String,
    request: EvmBalanceRequest,
    route_ref: ContentRef,
});

impl PortfolioCollectionDemand {
    fn new(
        correlation: String,
        request: EvmBalanceRequest,
        route_ref: ContentRef,
    ) -> Result<Self, PortfolioError> {
        let value = Self {
            correlation,
            request,
            route_ref,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), PortfolioError> {
        if !valid_public_text(&self.correlation, 256) || self.request.validate().is_err() {
            return Err(PortfolioError::InvalidValue);
        }
        Ok(())
    }
}

/// One singular domain-planned Portfolio admission value.
///
/// Its fields are intentionally private: callers can select a target and quote, but only the
/// trusted planner can assemble collection demand and route identities.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioSnapshotInput {
    portfolio_id: PortfolioId,
    collections: Vec<PortfolioCollectionDemand>,
    quote: QuoteCode,
    quotes: Vec<QuoteCode>,
    admission: Option<PortfolioAdmission>,
}

impl_checked_deserialize!(PortfolioSnapshotInput {
    portfolio_id: PortfolioId,
    collections: Vec<PortfolioCollectionDemand>,
    quote: QuoteCode,
    quotes: Vec<QuoteCode>,
    admission: Option<PortfolioAdmission>,
});

impl PortfolioSnapshotInput {
    fn from_demand(
        portfolio_id: PortfolioId,
        collections: Vec<PortfolioCollectionDemand>,
        quote: QuoteCode,
        quotes: Vec<QuoteCode>,
        admission: Option<PortfolioAdmission>,
    ) -> Result<Self, PortfolioError> {
        let value = Self {
            portfolio_id,
            collections,
            quote,
            quotes,
            admission,
        };
        value.validate()?;
        Ok(value)
    }

    /// Validates a decoded snapshot input and every declaration-ordered child request.
    fn validate(&self) -> Result<(), PortfolioError> {
        if !self.quotes.contains(&self.quote)
            || self.quotes.len() > 2
            || self
                .quotes
                .iter()
                .enumerate()
                .any(|(i, q)| self.quotes[..i].contains(q))
            || !valid_public_text(&self.portfolio_id.value, 256)
            || self.collections.is_empty()
            || self.collections.len() > PORTFOLIO_COLLECTION_LIMIT
            || self
                .collections
                .iter()
                .any(|collection| collection.validate().is_err())
            || total_sources(
                self.collections
                    .iter()
                    .map(|collection| &collection.request),
            ) > mfm_evm::EVM_BALANCE_SOURCE_LIMIT
            || duplicate_text(
                self.collections
                    .iter()
                    .map(|collection| collection.correlation.as_str()),
            )
            || duplicate_source_ids(
                self.collections
                    .iter()
                    .map(|collection| &collection.request),
            )
        {
            return Err(PortfolioError::InvalidValue);
        }
        Ok(())
    }

    /// Returns the immutable caller admission identity and enrichment linkage, if supplied.
    pub fn admission(&self) -> Option<&PortfolioAdmission> {
        self.admission.as_ref()
    }

    fn collection(&self, ordinal: usize) -> Option<&PortfolioCollectionDemand> {
        self.collections.get(ordinal)
    }
}

/// Complete cumulative Portfolio continuation passed between sequential States.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioContinuation {
    input: PortfolioSnapshotInput,
    completed_collections: Vec<PortfolioSnapshotCollection>,
}

impl_checked_deserialize!(PortfolioContinuation {
    input: PortfolioSnapshotInput,
    completed_collections: Vec<PortfolioSnapshotCollection>,
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioAnchor {
    number: String,
    hash: String,
}

impl_checked_deserialize!(PortfolioAnchor {
    number: String,
    hash: String,
});

impl PortfolioAnchor {
    fn new(number: String, hash: String) -> Result<Self, PortfolioError> {
        if !is_decimal_integer(&number) || !valid_public_text(&hash, 256) {
            return Err(PortfolioError::InvalidValue);
        }
        Ok(Self { number, hash })
    }

    fn validate(&self) -> Result<(), PortfolioError> {
        Self::new(self.number.clone(), self.hash.clone()).map(|_| ())
    }
}

impl PortfolioContinuation {
    fn new(
        input: PortfolioSnapshotInput,
        completed_collections: Vec<PortfolioSnapshotCollection>,
    ) -> Result<Self, PortfolioError> {
        let continuation = Self {
            input,
            completed_collections,
        };
        continuation.validate()?;
        Ok(continuation)
    }

    fn validate(&self) -> Result<(), PortfolioError> {
        self.input.validate()?;
        let next = self.completed_collections.len();
        if next > self.input.collections.len()
            || self
                .completed_collections
                .iter()
                .enumerate()
                .any(|(ordinal, result)| {
                    self.input.collection(ordinal).is_none_or(|demand| {
                        completed_collection_scaled_total(result, demand, ordinal as u32).is_none()
                    })
                })
        {
            return Err(PortfolioError::InvalidContinuation);
        }
        Ok(())
    }

    /// Returns the next collection ordinal derived from the completed prefix.
    fn next_collection_ordinal(&self) -> Option<u32> {
        (self.completed_collections.len() < self.input.collections.len())
            .then_some(self.completed_collections.len() as u32)
    }
}

fn completed_collection_scaled_total(
    result: &PortfolioSnapshotCollection,
    demand: &PortfolioCollectionDemand,
    ordinal: u32,
) -> Option<String> {
    if demand
        .request
        .sources()
        .first()
        .is_none_or(|source| result.chain_id != source.chain_id())
        || result.collection_ordinal != ordinal
        || result.anchor.validate().is_err()
        || result.holdings.len() != demand.request.sources().len()
        || result
            .holdings
            .iter()
            .zip(demand.request.sources())
            .any(|(holding, source)| {
                holding.source_id != source.source_id()
                    || holding.decimals > 30
                    || !is_decimal_integer(&holding.raw_units)
                    || holding.amount_dec != decimal_amount(&holding.raw_units, holding.decimals)
                    || !holding_matches_planned_source(holding, source)
            })
    {
        return None;
    }
    result
        .holdings
        .iter()
        .map(|holding| {
            demand
                .request
                .scale_units(&holding.raw_units, holding.decimals)
        })
        .collect::<Option<Vec<_>>>()
        .and_then(|amounts| sum_unsigned(&amounts))
}

fn holding_matches_planned_source(holding: &PortfolioHolding, source: &EvmBalanceSource) -> bool {
    match (&holding.asset, source.token()) {
        (PortfolioAsset::Native, None) => true,
        (PortfolioAsset::Token { contract }, Some(token)) => contract == &token.to_string(),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum PortfolioAsset {
    Native,
    Token { contract: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioHolding {
    pub source_id: String,
    pub asset: PortfolioAsset,
    pub decimals: u8,
    pub raw_units: String,
    pub amount_dec: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioSnapshotCollection {
    pub collection_ordinal: u32,
    pub chain_id: NonZeroU64,
    pub anchor: PortfolioAnchor,
    pub holdings: Vec<PortfolioHolding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioSnapshot {
    pub schema_version: u8,
    pub portfolio_id: PortfolioId,
    pub collections: Vec<PortfolioSnapshotCollection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioCollectionSummary {
    pub collection_ordinal: u32,
    pub total_value_dec: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioQuoteTotal {
    pub quote: QuoteCode,
    pub total_value_dec: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioReport {
    pub schema_version: u8,
    pub portfolio_id: PortfolioId,
    pub quote: QuoteCode,
    pub collection_summaries: Vec<PortfolioCollectionSummary>,
    pub totals_by_quote: Vec<PortfolioQuoteTotal>,
}

/// Final public Portfolio snapshot output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioSnapshotOutput {
    snapshot: PortfolioSnapshot,
    report: PortfolioReport,
}

impl_checked_deserialize!(PortfolioSnapshotOutput {
    snapshot: PortfolioSnapshot,
    report: PortfolioReport,
});

impl PortfolioSnapshotOutput {
    /// Validates the frozen snapshot and report projection agreement.
    fn validate(&self) -> Result<(), PortfolioError> {
        if self.snapshot.schema_version != 1
            || self.report.schema_version != 1
            || !valid_public_text(&self.snapshot.portfolio_id.value, 256)
            || self.snapshot.portfolio_id != self.report.portfolio_id
            || self.snapshot.collections.is_empty()
            || self.snapshot.collections.len() > PORTFOLIO_COLLECTION_LIMIT
            || self.report.collection_summaries.len() != self.snapshot.collections.len()
            || self.report.totals_by_quote.len() != 1
            || self.report.totals_by_quote[0].quote != self.report.quote
        {
            return Err(PortfolioError::InvalidValue);
        }
        let mut source_ids = BTreeSet::new();
        let mut totals = Vec::with_capacity(self.snapshot.collections.len());
        for (ordinal, collection) in self.snapshot.collections.iter().enumerate() {
            let Some(total) = collection_total_value_dec(collection, &mut source_ids) else {
                return Err(PortfolioError::InvalidValue);
            };
            let summary = &self.report.collection_summaries[ordinal];
            if collection.collection_ordinal != ordinal as u32
                || summary.collection_ordinal != ordinal as u32
                || summary.total_value_dec != total
            {
                return Err(PortfolioError::InvalidValue);
            }
            totals.push(total);
        }
        (sum_decimal_values(&totals)
            == Some(self.report.totals_by_quote[0].total_value_dec.clone()))
        .then_some(())
        .ok_or(PortfolioError::InvalidValue)
    }
}

fn collection_total_value_dec(
    collection: &PortfolioSnapshotCollection,
    source_ids: &mut BTreeSet<String>,
) -> Option<String> {
    if collection.anchor.validate().is_err() || collection.holdings.is_empty() {
        return None;
    }
    let mut values = Vec::with_capacity(collection.holdings.len());
    for holding in &collection.holdings {
        if !source_ids.insert(holding.source_id.clone())
            || !valid_public_text(&holding.source_id, 256)
            || holding.decimals > 30
            || !is_decimal_integer(&holding.raw_units)
            || holding.amount_dec != decimal_amount(&holding.raw_units, holding.decimals)
            || matches!(&holding.asset, PortfolioAsset::Token { contract }
                if !valid_public_text(contract, 128) || contract != &contract.to_ascii_lowercase())
        {
            return None;
        }
        values.push(canonical_decimal(holding.amount_dec.clone()));
    }
    sum_decimal_values(&values)
}

/// Explicit fail-fast Portfolio failure route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PortfolioSnapshotFailure {
    /// The admitted configuration is invalid.
    InvalidInput,
    /// One child collection failed and later children are suppressed.
    CollectionFailed {
        /// Declaration-ordered collection ordinal.
        ordinal: u16,
        /// Stable redacted collection failure code.
        code: String,
    },
    /// Final arithmetic or binding validation failed.
    ConsolidationFailed,
}

impl<'de> Deserialize<'de> for PortfolioSnapshotFailure {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            tag = "kind",
            content = "value",
            rename_all = "snake_case",
            deny_unknown_fields
        )]
        enum Wire {
            InvalidInput,
            CollectionFailed { ordinal: u16, code: String },
            ConsolidationFailed,
        }

        let value = match Wire::deserialize(deserializer)? {
            Wire::InvalidInput => Self::InvalidInput,
            Wire::CollectionFailed { ordinal, code } => Self::CollectionFailed { ordinal, code },
            Wire::ConsolidationFailed => Self::ConsolidationFailed,
        };
        value.validate().map(|_| value).map_err(de::Error::custom)
    }
}

impl PortfolioSnapshotFailure {
    fn validate(&self) -> Result<(), PortfolioError> {
        match self {
            Self::InvalidInput | Self::ConsolidationFailed => Ok(()),
            Self::CollectionFailed { ordinal, code }
                if usize::from(*ordinal) < PORTFOLIO_COLLECTION_LIMIT
                    && valid_public_text(code, 256) =>
            {
                Ok(())
            }
            Self::CollectionFailed { .. } => Err(PortfolioError::InvalidValue),
        }
    }
}

/// Initializes one Portfolio snapshot continuation.
pub struct InitializePortfolio;

/// Enters the next EVM balance collection.
pub struct EnterPortfolioCollection;

/// Resumes Portfolio aggregation after one EVM balance collection.
pub struct ResumePortfolioCollection;

/// Maps an original EVM balance failure into the Portfolio root failure contract.
pub struct MapEvmBalanceFailure;

/// Consolidates all completed collections into the Portfolio snapshot output.
pub struct ConsolidatePortfolio;

macro_rules! impl_portfolio_state {
    ($state:ident, $input:ty, $output:ty, $id:literal, $description:literal) => {
        impl $state {
            /// Stable State identity used by Program authoring and product inspection.
            pub const STATE_ID: &'static str = $id;
            /// Human-readable product inspection description.
            pub const DESCRIPTION: &'static str = $description;
        }

        impl State for $state {
            type Input = $input;
            type Output = $output;
            type Failure = PortfolioSnapshotFailure;

            fn state_id() -> mfm_program::Result<StableId> {
                StableId::new(Self::STATE_ID)
                    .map_err(|_| mfm_program::ProgramError::InvalidContract)
            }
        }
    };
}

mod enrichment;

impl_portfolio_state!(
    InitializePortfolio,
    PortfolioSnapshotInput,
    PortfolioContinuation,
    "mfm.portfolio.state.initialize@1",
    "Initializes one Portfolio snapshot continuation."
);
impl_portfolio_state!(
    EnterPortfolioCollection,
    PortfolioContinuation,
    EvmBalanceContext<PortfolioContinuation>,
    "mfm.portfolio.state.enter-collection@1",
    "Enters the next EVM balance collection."
);
impl_portfolio_state!(
    ResumePortfolioCollection,
    EvmBalanceCollectionCompletion<PortfolioContinuation>,
    PortfolioContinuation,
    "mfm.portfolio.state.resume-collection@1",
    "Resumes Portfolio aggregation after one EVM balance collection."
);
impl_portfolio_state!(
    ConsolidatePortfolio,
    PortfolioContinuation,
    PortfolioSnapshotOutput,
    "mfm.portfolio.state.consolidate@1",
    "Consolidates all completed collections into the Portfolio snapshot output."
);
fn initialize_portfolio(
    input: PortfolioSnapshotInput,
) -> ProposedStateOutcome<PortfolioContinuation, PortfolioSnapshotFailure> {
    match PortfolioContinuation::new(input, Vec::new()) {
        Ok(output) => portfolio_success(output),
        Err(_) => portfolio_failure(PortfolioSnapshotFailure::InvalidInput),
    }
}

fn enter_portfolio_collection(
    input: PortfolioContinuation,
) -> ProposedStateOutcome<EvmBalanceContext<PortfolioContinuation>, PortfolioSnapshotFailure> {
    let Some(ordinal) = input.next_collection_ordinal() else {
        return portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed);
    };
    let demand = match input.input.collection(ordinal as usize).cloned() {
        Some(demand) => demand,
        None => return portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed),
    };
    match EvmBalanceContext::new(
        demand.request,
        input,
        ordinal,
        demand.correlation,
        demand.route_ref,
    ) {
        Ok(output) => portfolio_success(output),
        Err(_) => portfolio_failure(PortfolioSnapshotFailure::InvalidInput),
    }
}

fn resume_portfolio_collection(
    input: EvmBalanceCollectionCompletion<PortfolioContinuation>,
) -> ProposedStateOutcome<PortfolioContinuation, PortfolioSnapshotFailure> {
    let (
        caller_context,
        collection_ordinal,
        chain_id,
        anchor_number,
        anchor_hash,
        balances,
        total_scaled,
    ) = input.into_parts();
    let mut continuation = caller_context;
    let expected = continuation.next_collection_ordinal();
    if expected != Some(collection_ordinal) {
        return portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed);
    }
    let anchor = match PortfolioAnchor::new(anchor_number, anchor_hash) {
        Ok(anchor) => anchor,
        Err(_) => return portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed),
    };
    let collection = PortfolioSnapshotCollection {
        collection_ordinal,
        chain_id,
        anchor,
        holdings: balances
            .into_iter()
            .map(
                |(source_id, _, token, decimals, raw_units)| PortfolioHolding {
                    source_id,
                    asset: match token {
                        None => PortfolioAsset::Native,
                        Some(contract) => PortfolioAsset::Token { contract },
                    },
                    decimals,
                    amount_dec: decimal_amount(&raw_units, decimals),
                    raw_units,
                },
            )
            .collect(),
    };
    let demand = match continuation.input.collection(collection_ordinal as usize) {
        Some(demand) => demand,
        None => return portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed),
    };
    if completed_collection_scaled_total(&collection, demand, collection_ordinal).as_deref()
        != Some(total_scaled.as_str())
    {
        return portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed);
    }
    continuation.completed_collections.push(collection);
    match continuation.validate() {
        Ok(()) => portfolio_success(continuation),
        Err(_) => portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed),
    }
}

impl mfm_program::ValueMap for MapEvmBalanceFailure {
    type Input = EvmBalanceFailure;
    type Output = PortfolioSnapshotFailure;
    type Params = mfm_program::NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.portfolio.map.evm-failure@1")
            .map_err(|_| mfm_program::ProgramError::InvalidContract)
    }
    fn apply(
        _: &Self::Params,
        input: EvmBalanceFailure,
    ) -> Result<PortfolioSnapshotFailure, mfm_program::StateExecutionError> {
        Ok(map_evm_balance_failure(input))
    }
}

fn map_evm_balance_failure(input: EvmBalanceFailure) -> PortfolioSnapshotFailure {
    let (collection_ordinal, code) = match input {
        EvmBalanceFailure::AnchorChanged {
            collection_ordinal, ..
        } => (collection_ordinal, "anchor_changed".to_owned()),
        EvmBalanceFailure::SourceUnavailable {
            collection_ordinal,
            code,
            ..
        }
        | EvmBalanceFailure::IntegrityBlocked {
            collection_ordinal,
            code,
            ..
        } => (collection_ordinal, code),
    };
    match u16::try_from(collection_ordinal) {
        Ok(ordinal) => PortfolioSnapshotFailure::CollectionFailed { ordinal, code },
        Err(_) => PortfolioSnapshotFailure::ConsolidationFailed,
    }
}

fn consolidate_portfolio(
    input: PortfolioContinuation,
) -> ProposedStateOutcome<PortfolioSnapshotOutput, PortfolioSnapshotFailure> {
    if input.next_collection_ordinal().is_some() || input.validate().is_err() {
        return portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed);
    }
    let mut summaries = Vec::with_capacity(input.completed_collections.len());
    let mut totals = Vec::with_capacity(input.completed_collections.len());
    let PortfolioContinuation {
        input,
        completed_collections,
    } = input;
    for (ordinal, collection) in completed_collections.iter().enumerate() {
        let Some(demand) = input.collection(ordinal) else {
            return portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed);
        };
        let Some(total_scaled) =
            completed_collection_scaled_total(collection, demand, ordinal as u32)
        else {
            return portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed);
        };
        let total_value_dec = decimal_amount(&total_scaled, demand.request.decimals());
        let total_value_dec = canonical_decimal(total_value_dec);
        totals.push(total_value_dec.clone());
        summaries.push(PortfolioCollectionSummary {
            collection_ordinal: ordinal as u32,
            total_value_dec,
        });
    }
    let aggregate = match sum_decimal_values(&totals) {
        Some(value) => value,
        None => return portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed),
    };
    let output = PortfolioSnapshotOutput {
        snapshot: PortfolioSnapshot {
            schema_version: 1,
            portfolio_id: input.portfolio_id.clone(),
            collections: completed_collections,
        },
        report: PortfolioReport {
            schema_version: 1,
            portfolio_id: input.portfolio_id,
            quote: input.quote.clone(),
            collection_summaries: summaries,
            totals_by_quote: vec![PortfolioQuoteTotal {
                quote: input.quote,
                total_value_dec: aggregate,
            }],
        },
    };
    match output.validate() {
        Ok(()) => portfolio_success(output),
        Err(_) => portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed),
    }
}

macro_rules! impl_portfolio_pure {
    ($state:ident, $evaluate:path) => {
        impl PureState for $state {
            fn evaluate(
                input: Self::Input,
            ) -> std::result::Result<
                ProposedStateOutcome<Self::Output, Self::Failure>,
                mfm_program::StateExecutionError,
            > {
                Ok($evaluate(input))
            }
        }
    };
}

impl_portfolio_pure!(InitializePortfolio, initialize_portfolio);
impl_portfolio_pure!(EnterPortfolioCollection, enter_portfolio_collection);
impl_portfolio_pure!(ResumePortfolioCollection, resume_portfolio_collection);

impl_portfolio_pure!(ConsolidatePortfolio, consolidate_portfolio);

fn portfolio_success<O, F>(output: O) -> ProposedStateOutcome<O, F> {
    ProposedStateOutcome::Success { output }
}

fn portfolio_failure<O, F>(failure: F) -> ProposedStateOutcome<O, F> {
    ProposedStateOutcome::Failure { failure }
}

fn decimal_amount(raw: &str, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_owned();
    }
    if raw.len() <= usize::from(decimals) {
        let mut value = String::from("0.");
        value.extend(std::iter::repeat_n('0', usize::from(decimals) - raw.len()));
        value.push_str(raw);
        return value;
    }
    let split = raw.len() - usize::from(decimals);
    format!("{}.{}", &raw[..split], &raw[split..])
}

fn canonical_decimal(mut value: String) -> String {
    if let Some((whole, fraction)) = value.split_once('.') {
        let fraction = fraction.trim_end_matches('0');
        value = if fraction.is_empty() {
            whole.to_owned()
        } else {
            format!("{whole}.{fraction}")
        };
    }
    value
}

fn sum_decimal_values(values: &[String]) -> Option<String> {
    let max_fraction = values
        .iter()
        .filter_map(|value| value.split_once('.').map(|(_, fraction)| fraction.len()))
        .max()
        .unwrap_or(0);
    let mut scaled = Vec::with_capacity(values.len());
    for value in values {
        let (whole, fraction) = match value.split_once('.') {
            Some((whole, fraction)) => (whole, fraction),
            None => (value.as_str(), ""),
        };
        if !is_decimal_integer(whole) || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let mut digits = String::with_capacity(whole.len() + max_fraction);
        digits.push_str(whole);
        digits.push_str(fraction);
        digits.extend(std::iter::repeat_n('0', max_fraction - fraction.len()));
        let digits = digits.trim_start_matches('0');
        scaled.push(if digits.is_empty() {
            "0".to_owned()
        } else {
            digits.to_owned()
        });
    }
    let total = sum_unsigned(&scaled)?;
    if max_fraction == 0 {
        return Some(total);
    }
    let padded = if total.len() <= max_fraction {
        format!("{}{}", "0".repeat(max_fraction + 1 - total.len()), total)
    } else {
        total
    };
    let split = padded.len() - max_fraction;
    Some(canonical_decimal(format!(
        "{}.{}",
        &padded[..split],
        &padded[split..]
    )))
}

fn sum_unsigned(values: &[String]) -> Option<String> {
    let mut digits = vec![0u8];
    for value in values {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let width = digits.len().max(value.len());
        digits.resize(width, 0);
        let mut carry = 0u16;
        for offset in 0..width {
            let right = value
                .as_bytes()
                .iter()
                .rev()
                .nth(offset)
                .map(|byte| u16::from(*byte - b'0'))
                .unwrap_or(0);
            let index = digits.len() - 1 - offset;
            let sum = u16::from(digits[index]) + right + carry;
            digits[index] = (sum % 10) as u8;
            carry = sum / 10;
        }
        if carry != 0 {
            digits.insert(0, carry as u8);
        }
        if digits.len() > 80 {
            return None;
        }
    }
    let output = digits
        .into_iter()
        .map(|digit| char::from(b'0' + digit))
        .collect::<String>();
    let output = output.trim_start_matches('0');
    Some(if output.is_empty() {
        "0".to_owned()
    } else {
        output.to_owned()
    })
}

/// Selector for selecting an admitted Portfolio target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioSnapshotSelector {
    target: PortfolioId,
    quote: QuoteCode,
}

impl_checked_deserialize!(PortfolioSnapshotSelector {
    target: PortfolioId,
    quote: QuoteCode,
});

impl PortfolioSnapshotSelector {
    fn validate(&self) -> Result<(), PortfolioError> {
        valid_public_text(&self.target.value, 256)
            .then_some(())
            .ok_or(PortfolioError::InvalidValue)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioCollectionConfig {
    pub correlation: String,
    pub request: EvmBalanceRequest,
}

/// Checked secret-free Portfolio snapshot authoring input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioConfig {
    portfolio_id: PortfolioId,
    quotes: Vec<QuoteCode>,
    collections: Vec<PortfolioCollectionConfig>,
}

impl_checked_deserialize!(PortfolioConfig {
    portfolio_id: PortfolioId,
    quotes: Vec<QuoteCode>,
    collections: Vec<PortfolioCollectionConfig>,
});
impl PortfolioConfig {
    fn validate(&self) -> Result<(), PortfolioError> {
        validate_config_parts(&self.portfolio_id, &self.quotes, self.collections.iter())
    }
}

fn validate_config_parts<'a>(
    portfolio_id: &PortfolioId,
    quotes: &[QuoteCode],
    collections: impl ExactSizeIterator<Item = &'a PortfolioCollectionConfig> + Clone,
) -> Result<(), PortfolioError> {
    if !valid_public_text(&portfolio_id.value, 256)
        || quotes.is_empty()
        || collections.len() == 0
        || collections.len() > PORTFOLIO_COLLECTION_LIMIT
        || collections.clone().any(|collection| {
            !valid_public_text(&collection.correlation, 256)
                || collection.request.validate().is_err()
        })
        || quotes
            .iter()
            .enumerate()
            .any(|(index, quote)| quotes[..index].contains(quote))
        || duplicate_text(
            collections
                .clone()
                .map(|collection| collection.correlation.as_str()),
        )
        || total_sources(collections.clone().map(|collection| &collection.request))
            > mfm_evm::EVM_BALANCE_SOURCE_LIMIT
        || duplicate_source_ids(collections.map(|collection| &collection.request))
    {
        return Err(PortfolioError::InvalidValue);
    }
    Ok(())
}

/// Selects trusted Portfolio authoring input and expands its exact Program and C0.
pub fn plan_snapshot(
    selector: PortfolioSnapshotSelector,
    config: &PortfolioConfig,
    targets: &[EvmPhysicalTarget],
    admission: Option<PortfolioAdmission>,
) -> Result<(Program, PortfolioSnapshotInput), PortfolioError> {
    plan::<ConsolidatePortfolio>(selector, config, targets, admission, entry_point_id()?)
}

fn plan<S>(
    selector: PortfolioSnapshotSelector,
    config: &PortfolioConfig,
    targets: &[EvmPhysicalTarget],
    admission: Option<PortfolioAdmission>,
    entry: EntryPointId,
) -> Result<(Program, PortfolioSnapshotInput), PortfolioError>
where
    S: PureState<Input = PortfolioContinuation, Failure = PortfolioSnapshotFailure>,
{
    if admission
        .as_ref()
        .is_some_and(|identity| identity.entry_point() != &entry)
    {
        return Err(PortfolioError::InvalidValue);
    }
    config.validate().map_err(|_| PortfolioError::Program)?;
    selector.validate()?;
    if selector.target != config.portfolio_id || !config.quotes.contains(&selector.quote) {
        return Err(PortfolioError::InvalidValue);
    }
    if targets.is_empty()
        || targets
            .windows(2)
            .any(|pair| pair[0].chain_id >= pair[1].chain_id)
    {
        return Err(PortfolioError::InvalidValue);
    }

    let required_chains = config
        .collections
        .iter()
        .filter_map(|collection| collection.request.sources().first())
        .map(EvmBalanceSource::chain_id)
        .collect::<BTreeSet<_>>();
    if required_chains.len() != targets.len()
        || !required_chains
            .iter()
            .copied()
            .eq(targets.iter().map(|target| target.chain_id))
    {
        return Err(PortfolioError::InvalidValue);
    }

    let mut demand = Vec::with_capacity(config.collections.len());
    for collection in &config.collections {
        let chain_id = collection
            .request
            .sources()
            .first()
            .map(EvmBalanceSource::chain_id)
            .ok_or(PortfolioError::Program)?;
        let target_index = targets
            .binary_search_by_key(&chain_id, |target| target.chain_id)
            .map_err(|_| PortfolioError::Program)?;
        let target = &targets[target_index];
        let route_ref = target.binding_ref().map_err(|_| PortfolioError::Program)?;
        demand.push(PortfolioCollectionDemand::new(
            collection.correlation.clone(),
            collection.request.clone(),
            route_ref.clone(),
        )?);
    }
    let input = PortfolioSnapshotInput::from_demand(
        config.portfolio_id.clone(),
        demand,
        selector.quote,
        config.quotes.clone(),
        admission,
    )?;
    let (conclusion_bound, child_bounds) = bounds::conclusion_bounds(&input)?;
    let checked_collections = input
        .collections
        .iter()
        .zip(child_bounds)
        .map(|(demand, bound)| {
            CollectEvmBalances::<PortfolioContinuation>::new(
                demand.route_ref.clone(),
                demand.request.clone(),
                bound,
            )
            .map_err(|_| PortfolioError::Program)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let root = PortfolioOperation::<S> {
        terminal: std::marker::PhantomData,
        checked_collections,
        conclusion_bound,
    };
    let program = expand_program(entry, &root, &input, mfm_program::ProgramLimits::new(0))
        .map_err(|_| PortfolioError::Program)?;
    Ok((program, input))
}

struct PortfolioOperation<S> {
    terminal: std::marker::PhantomData<fn() -> S>,
    checked_collections: Vec<CollectEvmBalances<PortfolioContinuation>>,
    conclusion_bound: mfm_program::ConclusionBound,
}

impl<S> Operation for PortfolioOperation<S>
where
    S: PureState<Input = PortfolioContinuation, Failure = PortfolioSnapshotFailure>,
{
    type Input = PortfolioSnapshotInput;
    type Output = S::Output;
    type Failure = PortfolioSnapshotFailure;

    fn validate_input(&self, input: &Self::Input) -> mfm_program::Result<()> {
        input
            .validate()
            .map_err(|_| mfm_program::ProgramError::InvalidContract)?;
        if input.collections.len() != self.checked_collections.len()
            || !input
                .collections
                .iter()
                .zip(&self.checked_collections)
                .all(|(demand, child)| child.matches_request(&demand.request, &demand.route_ref))
        {
            return Err(mfm_program::ProgramError::InvalidContract);
        }
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        use mfm_program::{Identity, NoParams, Occurrence};
        body.pure::<InitializePortfolio, Identity<Self::Failure>>(
            NoParams,
            Occurrence::new(),
            self.conclusion_bound,
        )?;
        for child in &self.checked_collections {
            body.pure::<EnterPortfolioCollection, Identity<Self::Failure>>(
                NoParams,
                Occurrence::new(),
                self.conclusion_bound,
            )?;
            body.operation::<CollectEvmBalances<PortfolioContinuation>, MapEvmBalanceFailure>(
                child, NoParams,
            )?;
            body.pure::<ResumePortfolioCollection, Identity<Self::Failure>>(
                NoParams,
                Occurrence::new(),
                self.conclusion_bound,
            )?;
        }
        body.pure::<S, Identity<Self::Failure>>(NoParams, Occurrence::new(), self.conclusion_bound)
    }
}

/// Redaction-safe Portfolio domain error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PortfolioError {
    /// A bounded public value is invalid.
    #[error("Portfolio domain value is invalid")]
    InvalidValue,
    /// A declaration-order continuation is not valid.
    #[error("Portfolio continuation is invalid")]
    InvalidContinuation,
    /// Program authoring or exact binding resolution failed.
    #[error("Portfolio Program contract is invalid")]
    Program,
}

fn entry_point_id() -> Result<EntryPointId, PortfolioError> {
    EntryPointId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).map_err(|_| PortfolioError::InvalidValue)
}

fn is_decimal_integer(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value.as_bytes().iter().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

fn valid_public_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
        && !string_contains_secret_marker(value)
}

fn total_sources<'a>(requests: impl Iterator<Item = &'a EvmBalanceRequest>) -> usize {
    requests
        .map(|request| request.sources().len())
        .try_fold(0usize, usize::checked_add)
        .unwrap_or(usize::MAX)
}

fn duplicate_text<'a>(mut values: impl Iterator<Item = &'a str>) -> bool {
    let mut values_seen = BTreeSet::new();
    values.any(|value| !values_seen.insert(value))
}

fn duplicate_source_ids<'a>(requests: impl Iterator<Item = &'a EvmBalanceRequest>) -> bool {
    let mut source_ids = BTreeSet::new();
    requests
        .flat_map(EvmBalanceRequest::sources)
        .any(|source| !source_ids.insert(source.source_id()))
}
