use mfm_evm::{
    CheckChainIdentity, CollectEvmBalances, ConfirmBalanceAnchor, ConsolidateBalanceCollection,
    EvmAnchorRead, EvmBalanceRead, EvmChainIdentityRead, ReadInitialAnchor, ReadNativeBalance,
    ReadTokenBalance, ReadTokenDecimals,
};
use mfm_portfolio::{
    ConsolidatePortfolio, EnterPortfolioCollection, InitializePortfolio, MapEvmBalanceFailure,
    PortfolioContinuation, ResumePortfolioCollection,
};
use mfm_runtime::RuntimeAssemblyBuilder;
use serde::Serialize;

use crate::config::ENTRY_POINTS;

/// Kind of one component admitted by the compiled product composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// Public executable configuration entry point.
    EntryPoint,
    /// Public reusable authoring Operation.
    Operation,
    /// Deterministic State without IO.
    PureState,
    /// Deterministic State driven by observational evidence.
    ReadState,
}

impl ComponentKind {
    /// Returns the stable text spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EntryPoint => "entry_point",
            Self::Operation => "operation",
            Self::PureState => "pure_state",
            Self::ReadState => "read_state",
        }
    }
}

/// One definition admitted by the compiled product composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ComponentSummary {
    kind: ComponentKind,
    id: &'static str,
    description: &'static str,
}

impl ComponentSummary {
    const fn new(kind: ComponentKind, id: &'static str, description: &'static str) -> Self {
        Self {
            kind,
            id,
            description,
        }
    }

    /// Returns the component kind.
    pub const fn kind(self) -> ComponentKind {
        self.kind
    }

    /// Returns the stable component identity.
    pub const fn id(self) -> &'static str {
        self.id
    }

    /// Returns the human-readable, non-semantic description.
    pub const fn description(self) -> &'static str {
        self.description
    }
}

type RegisterState = fn(&mut RuntimeAssemblyBuilder) -> mfm_runtime::Result<()>;

struct StateRegistration {
    summary: ComponentSummary,
    register: RegisterState,
}

macro_rules! pure_state {
    ($state:ty) => {
        StateRegistration {
            summary: ComponentSummary::new(
                ComponentKind::PureState,
                <$state>::STATE_ID,
                <$state>::DESCRIPTION,
            ),
            register: RuntimeAssemblyBuilder::register_pure::<$state>,
        }
    };
}

macro_rules! read_state {
    ($state:ty, $capability:ty) => {
        StateRegistration {
            summary: ComponentSummary::new(
                ComponentKind::ReadState,
                <$state>::STATE_ID,
                <$state>::DESCRIPTION,
            ),
            register: RuntimeAssemblyBuilder::register_read::<$state, $capability>,
        }
    };
}

const OPERATIONS: [ComponentSummary; 1] = [ComponentSummary::new(
    ComponentKind::Operation,
    CollectEvmBalances::<PortfolioContinuation>::OPERATION_ID,
    CollectEvmBalances::<PortfolioContinuation>::DESCRIPTION,
)];

const STATES: [StateRegistration; 12] = [
    pure_state!(mfm_portfolio::ResolvePortfolioAssets),
    pure_state!(InitializePortfolio),
    pure_state!(EnterPortfolioCollection),
    pure_state!(ResumePortfolioCollection),
    pure_state!(ConsolidatePortfolio),
    read_state!(
        CheckChainIdentity<PortfolioContinuation>,
        EvmChainIdentityRead
    ),
    read_state!(ReadInitialAnchor<PortfolioContinuation>, EvmAnchorRead),
    read_state!(ReadNativeBalance<PortfolioContinuation>, EvmBalanceRead),
    read_state!(ReadTokenDecimals<PortfolioContinuation>, EvmBalanceRead),
    read_state!(ReadTokenBalance<PortfolioContinuation>, EvmBalanceRead),
    read_state!(ConfirmBalanceAnchor<PortfolioContinuation>, EvmAnchorRead),
    pure_state!(ConsolidateBalanceCollection<PortfolioContinuation>),
];

pub(crate) fn register_states(builder: &mut RuntimeAssemblyBuilder) -> mfm_runtime::Result<()> {
    for state in &STATES {
        (state.register)(builder)?;
    }
    builder.register_map::<MapEvmBalanceFailure>()
}

pub(crate) fn components() -> Vec<ComponentSummary> {
    let mut components = Vec::with_capacity(ENTRY_POINTS.len() + OPERATIONS.len() + STATES.len());
    components.extend(ENTRY_POINTS.iter().map(|entry_point| {
        ComponentSummary::new(
            ComponentKind::EntryPoint,
            entry_point.entry_point(),
            entry_point.description(),
        )
    }));
    components.extend(OPERATIONS);
    components.extend(STATES.iter().map(|state| state.summary));
    components
        .sort_unstable_by(|left, right| (left.kind(), left.id()).cmp(&(right.kind(), right.id())));
    components
}
