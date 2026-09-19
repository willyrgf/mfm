//! Maintained typed source shapes; admission owns demand and resources own native selection.
use super::*;
use mfm_chain::balance::{BalanceSourceDefinition, ConsolidateBalanceCollection};
use mfm_program::{Operation, OperationDefinition, Plan, Pure};
use mfm_values::MfmValue as Value;

fn sources<K: Value>(
    demand: &PortfolioCollectionDemand,
) -> mfm_program::Result<Vec<Operation<BalanceSourceDefinition<K>>>> {
    demand.validate().map_err(|source| {
        mfm_program::ProgramError::Diagnostic(source.into_diagnostic("plan_portfolio_collection"))
    })?;
    demand
        .request
        .sources()
        .iter()
        .zip(&demand.executions)
        .enumerate()
        .map(|(ordinal, (source, execution))| {
            Ok(Operation::new(BalanceSourceDefinition::new(
                source.clone(),
                ordinal as u32,
                demand.request.decimals(),
                execution.clone(),
            )))
        })
        .collect()
}

macro_rules! collection {
    ($definition:ident, $caller:ty, $enter:ty, $resume:ty) => {
        /// One maintained collection occurrence derived from admitted demand.
        pub struct $definition {
            demand: PortfolioCollectionDemand,
        }
        impl OperationDefinition for $definition {
            type Body = (
                Pure<$enter>,
                Vec<Operation<BalanceSourceDefinition<$caller>>>,
                Pure<ConsolidateBalanceCollection<$caller>>,
                Pure<$resume>,
            );
        }
        impl<Parent: ?Sized> Plan<Parent> for $definition {
            type Config = Self;
            fn plan<'a>(&'a self, _: &'a Parent) -> mfm_program::Result<(&'a Self, Self::Body)> {
                Ok((
                    self,
                    (
                        Pure::default(),
                        sources(&self.demand)?,
                        Pure::default(),
                        Pure::default(),
                    ),
                ))
            }
        }
    };
}
collection!(
    SnapshotCollectionDefinition,
    PortfolioContinuation,
    EnterPortfolioCollection,
    ResumePortfolioCollection
);
collection!(
    EnrichmentCollectionDefinition,
    EnrichmentContinuation,
    EnterEnrichmentCollection,
    ResumeEnrichmentCollection
);

/// Maintained snapshot source inferred from checked Portfolio admission.
pub type PortfolioSnapshotOperation = Operation<SnapshotDefinition>;
/// Maintained enrichment source with its own checked continuation.
pub type PortfolioEnrichmentOperation = Operation<EnrichmentDefinition>;
/// Snapshot structure independent of occurrence count and live resources.
#[derive(Default)]
pub struct SnapshotDefinition;
impl OperationDefinition for SnapshotDefinition {
    fn metadata() -> Option<(&'static str, &'static str)> {
        Some((
            "mfm.portfolio.operation.snapshot@1",
            "Collects and consolidates a checked Portfolio snapshot.",
        ))
    }
    type Body = (
        Pure<InitializePortfolio>,
        Vec<Operation<SnapshotCollectionDefinition>>,
        Pure<ConsolidatePortfolio>,
    );
}
impl Plan<PortfolioSnapshotInput> for SnapshotDefinition {
    type Config = PortfolioSnapshotInput;
    fn plan<'a>(
        &'a self,
        input: &'a Self::Config,
    ) -> mfm_program::Result<(&'a Self::Config, Self::Body)> {
        input.validate().map_err(|source| {
            mfm_program::ProgramError::Diagnostic(source.into_diagnostic("plan_portfolio_snapshot"))
        })?;
        Ok((
            input,
            (
                Pure::default(),
                input
                    .collections
                    .iter()
                    .cloned()
                    .map(|demand| Operation::new(SnapshotCollectionDefinition { demand }))
                    .collect(),
                Pure::default(),
            ),
        ))
    }
}
/// Enrichment structure independent of occurrence count and live resources.
#[derive(Default)]
pub struct EnrichmentDefinition;
impl OperationDefinition for EnrichmentDefinition {
    fn metadata() -> Option<(&'static str, &'static str)> {
        Some((
            "mfm.portfolio.operation.enrichment@1",
            "Observes configured candidates and retains required or nonzero sources.",
        ))
    }
    type Body = (
        Pure<InitializeEnrichment>,
        Vec<Operation<EnrichmentCollectionDefinition>>,
        Pure<ResolvePortfolioAssets>,
    );
}
impl Plan<PortfolioEnrichmentInput> for EnrichmentDefinition {
    type Config = PortfolioEnrichmentInput;
    fn plan<'a>(
        &'a self,
        input: &'a Self::Config,
    ) -> mfm_program::Result<(&'a Self::Config, Self::Body)> {
        input.validate().map_err(|source| {
            mfm_program::ProgramError::Diagnostic(
                source.into_diagnostic("plan_portfolio_enrichment"),
            )
        })?;
        Ok((
            input,
            (
                Pure::default(),
                input
                    .input
                    .collections
                    .iter()
                    .cloned()
                    .map(|demand| Operation::new(EnrichmentCollectionDefinition { demand }))
                    .collect(),
                Pure::default(),
            ),
        ))
    }
}
