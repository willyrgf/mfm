use mfm_evm_live::client::portfolio::PortfolioResources;
use serde::Serialize;

use crate::config::ENTRY_POINTS;

/// Kind of one compiled product component or supported transport entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// Public executable configuration entry point.
    EntryPoint,
    /// Reusable authoring Operation.
    Operation,
    /// Deterministic State without IO.
    PureState,
    /// Observational State.
    ReadState,
    /// Mutating State.
    EffectState,
}
impl ComponentKind {
    /// Stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EntryPoint => "entry_point",
            Self::Operation => "operation",
            Self::PureState => "pure_state",
            Self::ReadState => "read_state",
            Self::EffectState => "effect_state",
        }
    }
}
/// Product projection of metadata discovered from its installed sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentSummary {
    kind: ComponentKind,
    id: String,
    description: &'static str,
}
impl ComponentSummary {
    /// Component kind.
    pub fn kind(&self) -> ComponentKind {
        self.kind
    }
    /// Stable definition identity.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Non-semantic owner description.
    pub fn description(&self) -> &'static str {
        self.description
    }
}

pub(crate) fn components() -> mfm_program::Result<Vec<ComponentSummary>> {
    let mut components = mfm_program::components::<PortfolioResources>()?
        .into_iter()
        .map(|component| {
            let kind = match component.kind() {
                mfm_program::ComponentKind::Operation => ComponentKind::Operation,
                mfm_program::ComponentKind::PureState => ComponentKind::PureState,
                mfm_program::ComponentKind::ReadState => ComponentKind::ReadState,
                mfm_program::ComponentKind::EffectState => ComponentKind::EffectState,
            };
            ComponentSummary {
                kind,
                id: component.id().as_str().to_owned(),
                description: component.description(),
            }
        })
        .collect::<Vec<_>>();
    components.extend(ENTRY_POINTS.iter().map(|entry| ComponentSummary {
        kind: ComponentKind::EntryPoint,
        id: entry.entry_point().to_owned(),
        description: entry.description(),
    }));
    components.sort_unstable_by(|left, right| (left.kind, &left.id).cmp(&(right.kind, &right.id)));
    Ok(components)
}
