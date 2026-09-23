#![warn(missing_docs)]
//! Secret-free sequential Portfolio State values.
//!
//! Portfolio owns one singular admission context and one cumulative continuation. Each shared
//! collection is expanded into an ordinary sequential child State; there is no runtime collection
//! loop, output map, parallel branch, or multi-result join.

pub use enrichment::{
    EnrichmentCollection, EnrichmentContinuation, EnrichmentProvenance, EnterEnrichmentCollection,
    InitializeEnrichment, PortfolioAdmission, PortfolioEnrichmentInput, PortfolioEnrichmentOutput,
    ResolvePortfolioAssets, ResumeEnrichmentCollection, PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID,
};
use mfm_chain::balance::BalanceFailureCode;

use std::collections::BTreeSet;

use mfm_chain::balance::{
    BalanceCollectionCompletion, BalanceCollectionMetadata, BalanceContext, BalanceExecutionConfig,
    BalanceRequest, ConfirmedBalance, DecimalScale,
};
use mfm_ids::{ContentRef, EntryPointId, StableId};
use mfm_program::{Never, ProposedStateOutcome, PureState, State};
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
    "Builds a Portfolio snapshot from confirmed balance observations.";
/// Maximum declaration-ordered EVM collections.
const PORTFOLIO_COLLECTION_LIMIT: usize = 64;
const PORTFOLIO_SOURCE_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
/// Supported semantic Portfolio quote currencies.
pub enum QuoteCode {
    /// United States dollar.
    Usd,
    /// Euro.
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

/// Checked collection request and one exact native execution descriptor per declared source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioCollectionDemand {
    correlation: String,
    request: BalanceRequest,
    executions: Vec<BalanceExecutionConfig>,
}
impl_checked_deserialize!(PortfolioCollectionDemand {
    correlation: String, request: BalanceRequest, executions: Vec<BalanceExecutionConfig>,
});
impl PortfolioCollectionDemand {
    /// Checks descriptor coverage and common caller-owned route without interpreting native data.
    pub fn new(
        correlation: String,
        request: BalanceRequest,
        executions: Vec<BalanceExecutionConfig>,
    ) -> Result<Self, PortfolioError> {
        let value = Self {
            correlation,
            request,
            executions,
        };
        value.validate()?;
        Ok(value)
    }
    /// Caller correlation preserved through collection handoff.
    pub fn correlation(&self) -> &str {
        &self.correlation
    }
    /// Exact declaration-ordered semantic collection request.
    pub fn request(&self) -> &BalanceRequest {
        &self.request
    }
    /// Execution descriptors in source order, retained for native binding and cold publication.
    pub fn executions(&self) -> &[BalanceExecutionConfig] {
        &self.executions
    }
    /// The common expected route; no native Object decoding occurs here.
    pub fn route_ref(&self) -> Result<&ContentRef, PortfolioError> {
        self.executions
            .first()
            .map(BalanceExecutionConfig::route_ref)
            .ok_or(PortfolioError::InvalidValue)
    }
    fn validate(&self) -> Result<(), PortfolioError> {
        if !valid_public_text(&self.correlation, 256) {
            return Err(PortfolioError::InvalidValue);
        }
        if self.executions.len() != self.request.sources().len() {
            return Err(PortfolioError::ExecutionCount {
                expected: self.request.sources().len(),
                actual: self.executions.len(),
            });
        }
        let expected = self.route_ref()?;
        for (ordinal, execution) in self.executions.iter().enumerate() {
            if execution.route_ref() != expected {
                return Err(PortfolioError::ExecutionRoute {
                    ordinal,
                    expected: Box::new(expected.clone()),
                    actual: Box::new(execution.route_ref().clone()),
                });
            }
        }
        Ok(())
    }
}

fn validate_demand_parts<'a>(
    portfolio_id: &PortfolioId,
    quote: &QuoteCode,
    quotes: &[QuoteCode],
    collections: impl ExactSizeIterator<Item = &'a PortfolioCollectionDemand> + Clone,
) -> Result<(), PortfolioError> {
    if !quotes.contains(quote)
        || quotes.len() > 2
        || quotes
            .iter()
            .enumerate()
            .any(|(index, quote)| quotes[..index].contains(quote))
        || !valid_public_text(&portfolio_id.value, 256)
    {
        return Err(PortfolioError::InvalidValue);
    }
    validate_collection_set(
        collections
            .clone()
            .map(|collection| collection.correlation.as_str()),
        collections
            .flat_map(|collection| collection.request.sources())
            .map(mfm_chain::balance::BalanceSource::source_id),
    )
}

fn validate_collection_set<'a>(
    correlations: impl ExactSizeIterator<Item = &'a str>,
    source_ids: impl Iterator<Item = &'a str>,
) -> Result<(), PortfolioError> {
    if correlations.len() == 0
        || correlations.len() > PORTFOLIO_COLLECTION_LIMIT
        || duplicate_text(correlations)
    {
        return Err(PortfolioError::InvalidValue);
    }
    let mut seen = BTreeSet::new();
    for source_id in source_ids {
        if !seen.insert(source_id) || seen.len() > PORTFOLIO_SOURCE_LIMIT {
            return Err(PortfolioError::InvalidValue);
        }
    }
    Ok(())
}

/// One singular domain-planned Portfolio admission value.
///
/// Its fields are intentionally private: callers can select a target and quote, but only the
/// trusted planner can assemble collection demand and route identities.
#[derive(Debug, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "snapshot-input",
    version = "2",
    schema = "mfm.portfolio-snapshot-input"
)]
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
    /// Admits checked semantic demand supplied by a native client.
    pub fn new(
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

    /// Requested Portfolio identity.
    pub fn portfolio_id(&self) -> &PortfolioId {
        &self.portfolio_id
    }
    /// Declared semantic collections.
    pub fn collections(&self) -> &[PortfolioCollectionDemand] {
        &self.collections
    }
    /// Selected quote currency.
    pub fn quote(&self) -> &QuoteCode {
        &self.quote
    }
    /// Supported quote currencies retained by admission.
    pub fn quotes(&self) -> &[QuoteCode] {
        &self.quotes
    }

    /// Checks Portfolio-wide relationships among the already checked child requests.
    fn validate(&self) -> Result<(), PortfolioError> {
        validate_demand_parts(
            &self.portfolio_id,
            &self.quote,
            &self.quotes,
            self.collections.iter(),
        )
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
#[mfm(
    namespace = "mfm.portfolio",
    name = "continuation",
    version = "2",
    schema = "mfm.portfolio-continuation"
)]
pub struct PortfolioContinuation {
    input: PortfolioSnapshotInput,
    completed_collections: Vec<PortfolioSnapshotCollection>,
}

impl_checked_deserialize!(PortfolioContinuation {
    input: PortfolioSnapshotInput,
    completed_collections: Vec<PortfolioSnapshotCollection>,
});

impl PortfolioContinuation {
    fn new(input: PortfolioSnapshotInput) -> Self {
        Self {
            input,
            completed_collections: Vec::new(),
        }
    }

    fn validate(&self) -> Result<(), PortfolioError> {
        let next = self.completed_collections.len();
        if next > self.input.collections.len() {
            return Err(PortfolioError::InvalidContinuation);
        }
        for (ordinal, result) in self.completed_collections.iter().enumerate() {
            let demand = self
                .input
                .collection(ordinal)
                .ok_or(PortfolioError::InvalidContinuation)?;
            validate_completed_collection(result, demand, ordinal as u32)?;
        }
        Ok(())
    }

    /// Returns the next collection ordinal derived from the completed prefix.
    fn next_collection_ordinal(&self) -> Option<u32> {
        (self.completed_collections.len() < self.input.collections.len())
            .then_some(self.completed_collections.len() as u32)
    }
}

fn validate_completed_collection(
    result: &PortfolioSnapshotCollection,
    demand: &PortfolioCollectionDemand,
    ordinal: u32,
) -> Result<(), PortfolioError> {
    if result.metadata.collection_ordinal() != ordinal
        || result.metadata.correlation() != demand.correlation
        || result.metadata.route_ref() != demand.route_ref()?
        || result.decimals != demand.request.decimals()
        || result.executions != demand.executions
        || !result
            .balances
            .iter()
            .map(ConfirmedBalance::source)
            .eq(demand.request.sources())
    {
        return Err(PortfolioError::InvalidContinuation);
    }
    Ok(())
}

/// Confirmed shared balances and checked aggregate, without native presentation fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioSnapshotCollection {
    metadata: BalanceCollectionMetadata,
    decimals: DecimalScale,
    balances: Vec<ConfirmedBalance>,
    executions: Vec<BalanceExecutionConfig>,
    total_scaled: String,
}
impl_checked_deserialize!(PortfolioSnapshotCollection {
    metadata: BalanceCollectionMetadata, decimals: DecimalScale, balances: Vec<ConfirmedBalance>,
    executions: Vec<BalanceExecutionConfig>, total_scaled: String,
});
impl PortfolioSnapshotCollection {
    fn validate(&self) -> Result<(), PortfolioError> {
        let request = BalanceRequest::new(
            self.balances
                .iter()
                .map(|balance| balance.source().clone())
                .collect(),
            self.decimals,
        )?;
        request.validate_confirmed(&self.balances)?;
        let demand = PortfolioCollectionDemand::new(
            self.metadata.correlation().to_owned(),
            request,
            self.executions.clone(),
        )?;
        if demand.route_ref()? != self.metadata.route_ref()
            || demand.request.total_scaled(
                self.balances
                    .iter()
                    .map(|entry| (entry.raw_units(), entry.source_decimals())),
            )? != self.total_scaled
        {
            return Err(PortfolioError::InvalidContinuation);
        }
        Ok(())
    }
    /// Checked collection occurrence metadata.
    pub fn metadata(&self) -> &BalanceCollectionMetadata {
        &self.metadata
    }
    /// Requested aggregate scale.
    pub fn decimals(&self) -> DecimalScale {
        self.decimals
    }
    /// Confirmed source observations in declaration order.
    pub fn balances(&self) -> &[ConfirmedBalance] {
        &self.balances
    }
    /// Source-aligned native execution descriptors.
    pub fn executions(&self) -> &[BalanceExecutionConfig] {
        &self.executions
    }
    /// Checked aggregate in collection-scaled integer units.
    pub fn total_scaled(&self) -> &str {
        &self.total_scaled
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct PortfolioSnapshot {
    schema_version: u8,
    portfolio_id: PortfolioId,
    collections: Vec<PortfolioSnapshotCollection>,
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
/// Checked semantic Portfolio aggregate report.
pub struct PortfolioReport {
    schema_version: u8,
    portfolio_id: PortfolioId,
    quote: QuoteCode,
    collection_summaries: Vec<PortfolioCollectionSummary>,
    totals_by_quote: Vec<PortfolioQuoteTotal>,
}

/// Final public Portfolio snapshot output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "snapshot-output",
    version = "2",
    schema = "mfm.portfolio-snapshot-output"
)]
pub struct PortfolioSnapshotOutput {
    snapshot: PortfolioSnapshot,
    report: PortfolioReport,
}

impl_checked_deserialize!(PortfolioSnapshotOutput {
    snapshot: PortfolioSnapshot,
    report: PortfolioReport,
});

impl PortfolioSnapshotOutput {
    /// Snapshot Portfolio identity.
    pub fn portfolio_id(&self) -> &PortfolioId {
        &self.snapshot.portfolio_id
    }
    /// Confirmed semantic collections, in declared order.
    pub fn collections(&self) -> &[PortfolioSnapshotCollection] {
        &self.snapshot.collections
    }
    /// Checked semantic report, independent of native rendering.
    pub fn report(&self) -> &PortfolioReport {
        &self.report
    }
    /// Validates the frozen snapshot and report projection agreement.
    fn validate(&self) -> Result<(), PortfolioError> {
        if self.snapshot.schema_version != 1
            || self.report.schema_version != 1
            || !valid_public_text(&self.snapshot.portfolio_id.value, 256)
            || self.snapshot.portfolio_id != self.report.portfolio_id
            || self.report.collection_summaries.len() != self.snapshot.collections.len()
            || self.report.totals_by_quote.len() != 1
            || self.report.totals_by_quote[0].quote != self.report.quote
        {
            return Err(PortfolioError::InvalidValue);
        }
        validate_collection_set(
            self.snapshot
                .collections
                .iter()
                .map(|collection| collection.metadata.correlation()),
            self.snapshot
                .collections
                .iter()
                .flat_map(|collection| &collection.balances)
                .map(|balance| balance.source().source_id()),
        )?;
        let mut totals = Vec::with_capacity(self.snapshot.collections.len());
        for (ordinal, collection) in self.snapshot.collections.iter().enumerate() {
            let total = canonical_decimal(decimal_amount(
                &collection.total_scaled,
                collection.decimals.get(),
            ));
            let summary = &self.report.collection_summaries[ordinal];
            if collection.metadata.collection_ordinal() != ordinal as u32
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

/// Product failure summary projected from an exact retained original.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PortfolioSnapshotFailure {
    /// One child collection failed and later children are suppressed.
    CollectionFailed {
        /// Declaration-ordered collection ordinal.
        ordinal: u16,
        /// Stable redacted collection failure code.
        code: BalanceFailureCode,
    },
    /// Final decimal aggregation exceeded capacity.
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
            CollectionFailed {
                ordinal: u16,
                code: BalanceFailureCode,
            },
            ConsolidationFailed,
        }

        let value = match Wire::deserialize(deserializer)? {
            Wire::CollectionFailed { ordinal, code } => Self::CollectionFailed { ordinal, code },
            Wire::ConsolidationFailed => Self::ConsolidationFailed,
        };
        value.validate().map(|_| value).map_err(de::Error::custom)
    }
}

impl PortfolioSnapshotFailure {
    fn validate(&self) -> Result<(), PortfolioError> {
        match self {
            Self::ConsolidationFailed => Ok(()),
            Self::CollectionFailed { ordinal, .. }
                if usize::from(*ordinal) < PORTFOLIO_COLLECTION_LIMIT =>
            {
                Ok(())
            }
            Self::CollectionFailed { .. } => Err(PortfolioError::InvalidValue),
        }
    }
}

/// Exact original when Portfolio cannot represent the consolidated decimal aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "consolidation-failure",
    version = "1",
    schema = "mfm.portfolio-consolidation-failure"
)]
pub enum PortfolioConsolidationFailure {
    /// Cross-collection decimal alignment or addition exceeds aggregate capacity.
    AggregateCapacityExceeded,
}

impl mfm_program::ClassifyError for PortfolioConsolidationFailure {
    fn classify(&self) -> mfm_program::Classification {
        mfm_program::Classification::Permanent
    }
}

/// Initializes one Portfolio snapshot continuation.
pub struct InitializePortfolio;

/// Enters the next balance collection.
pub struct EnterPortfolioCollection;

/// Resumes Portfolio aggregation after one confirmed balance collection.
pub struct ResumePortfolioCollection;

/// Consolidates all completed collections into the Portfolio snapshot output.
pub struct ConsolidatePortfolio;

macro_rules! impl_portfolio_state {
    ($state:ident, $input:ty, $output:ty, $failure:ty, $id:literal, $description:literal) => {
        impl $state {
            /// Stable State identity used by Program authoring and product inspection.
            pub const STATE_ID: &'static str = $id;
            /// Human-readable product inspection description.
            pub const DESCRIPTION: &'static str = $description;
        }

        impl State for $state {
            fn description() -> &'static str {
                Self::DESCRIPTION
            }
            type Input = $input;
            type Output = $output;
            type Failure = $failure;

            fn state_id() -> mfm_program::Result<StableId> {
                Ok(StableId::new(Self::STATE_ID)?)
            }
        }
    };
}

mod enrichment;

impl_portfolio_state!(
    InitializePortfolio,
    PortfolioSnapshotInput,
    PortfolioContinuation,
    Never,
    "mfm.portfolio.state.initialize@1",
    "Initializes one Portfolio snapshot continuation."
);
impl_portfolio_state!(
    EnterPortfolioCollection,
    PortfolioContinuation,
    BalanceContext<PortfolioContinuation>,
    Never,
    "mfm.portfolio.state.enter-collection@1",
    "Enters the next balance collection."
);
impl_portfolio_state!(
    ResumePortfolioCollection,
    BalanceCollectionCompletion<PortfolioContinuation>,
    PortfolioContinuation,
    Never,
    "mfm.portfolio.state.resume-collection@1",
    "Resumes Portfolio aggregation after one confirmed balance collection."
);
impl_portfolio_state!(
    ConsolidatePortfolio,
    PortfolioContinuation,
    PortfolioSnapshotOutput,
    PortfolioConsolidationFailure,
    "mfm.portfolio.state.consolidate@1",
    "Consolidates all completed collections into the Portfolio snapshot output."
);
fn initialize_portfolio(
    input: PortfolioSnapshotInput,
) -> Result<ProposedStateOutcome<PortfolioContinuation, Never>, mfm_values::InvocationDiagnostic> {
    Ok(portfolio_success(PortfolioContinuation::new(input)))
}

mod collection;

fn consolidate_portfolio(
    input: PortfolioContinuation,
) -> Result<
    ProposedStateOutcome<PortfolioSnapshotOutput, PortfolioConsolidationFailure>,
    mfm_values::InvocationDiagnostic,
> {
    if input.next_collection_ordinal().is_some() {
        return Err(PortfolioError::InvalidContinuation.into_diagnostic("consolidate_portfolio"));
    }
    let mut summaries = Vec::with_capacity(input.completed_collections.len());
    let mut totals = Vec::with_capacity(input.completed_collections.len());
    let PortfolioContinuation {
        input,
        completed_collections,
    } = input;
    for (ordinal, collection) in completed_collections.iter().enumerate() {
        let total_value_dec = decimal_amount(&collection.total_scaled, collection.decimals.get());
        let total_value_dec = canonical_decimal(total_value_dec);
        totals.push(total_value_dec.clone());
        summaries.push(PortfolioCollectionSummary {
            collection_ordinal: ordinal as u32,
            total_value_dec,
        });
    }
    let aggregate = match sum_decimal_values(&totals) {
        Some(value) => value,
        None => {
            return Ok(portfolio_failure(
                PortfolioConsolidationFailure::AggregateCapacityExceeded,
            ))
        }
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
    output
        .validate()
        .map_err(|source| source.into_diagnostic("consolidate_portfolio"))?;
    Ok(portfolio_success(output))
}

macro_rules! impl_portfolio_pure {
    ($state:ident, $evaluate:path) => {
        impl PureState for $state {
            fn evaluate(
                input: Self::Input,
            ) -> std::result::Result<
                ProposedStateOutcome<Self::Output, Self::Failure>,
                mfm_values::InvocationDiagnostic,
            > {
                $evaluate(input)
            }
        }
    };
}

impl_portfolio_pure!(InitializePortfolio, initialize_portfolio);

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
    /// Checks a target and semantic quote selection.
    pub fn new(target: PortfolioId, quote: QuoteCode) -> Result<Self, PortfolioError> {
        let value = Self { target, quote };
        value.validate()?;
        Ok(value)
    }
    /// Requested Portfolio target.
    pub fn target(&self) -> &PortfolioId {
        &self.target
    }
    /// Requested quote currency.
    pub fn quote(&self) -> &QuoteCode {
        &self.quote
    }
    fn validate(&self) -> Result<(), PortfolioError> {
        valid_public_text(&self.target.value, 256)
            .then_some(())
            .ok_or(PortfolioError::InvalidValue)
    }
}

mod planning;
pub use planning::{PortfolioEnrichmentOperation, PortfolioSnapshotOperation};

/// Redaction-safe Portfolio domain error.
#[derive(Debug, Serialize, thiserror::Error)]
pub enum PortfolioError {
    /// Execution descriptors must cover the exact declared source sequence.
    #[error("Portfolio execution descriptor count {actual} differs from {expected}")]
    ExecutionCount {
        /// Declared source count.
        expected: usize,
        /// Retained descriptor count.
        actual: usize,
    },
    /// Every descriptor in a collection must retain the same caller-owned route expectation.
    #[error("Portfolio source {ordinal} execution route differs from its collection")]
    ExecutionRoute {
        /// Source index in the collection.
        ordinal: usize,
        /// Collection's expected route.
        expected: Box<ContentRef>,
        /// Source's conflicting route.
        actual: Box<ContentRef>,
    },
    /// Confirmed shared facts failed their owning collection contract.
    #[error("Portfolio confirmed balance context is invalid: {0}")]
    Context(#[from] mfm_chain::balance::BalanceContextError),
    /// Shared arithmetic rejected retained collection facts.
    #[error("Portfolio collection arithmetic failed: {0}")]
    Arithmetic(#[from] mfm_chain::balance::BalanceCollectionFailure),
    /// Shared collection admission rejected the supplied configuration.
    #[error("Portfolio balance request is invalid: {0}")]
    BalanceRequest(#[from] mfm_chain::balance::BalanceRequestError),
    /// A bounded public value is invalid.
    #[error("Portfolio domain value is invalid")]
    InvalidValue,
    /// A declaration-order continuation is not valid.
    #[error("Portfolio continuation is invalid")]
    InvalidContinuation,
}
impl PortfolioError {
    fn into_diagnostic(self, operation: &'static str) -> mfm_values::InvocationDiagnostic {
        mfm_values::InvocationDiagnostic::from_fields("state_internal", operation, &self, None)
    }
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

fn duplicate_text<'a>(mut values: impl Iterator<Item = &'a str>) -> bool {
    let mut values_seen = BTreeSet::new();
    values.any(|value| !values_seen.insert(value))
}

#[cfg(test)]
mod snapshot_extremes;
