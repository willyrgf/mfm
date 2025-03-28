use anyhow::{anyhow, Error, Result};
use std::{fmt, sync::Arc};

pub mod context;
pub mod safe_context;

use self::safe_context::SafeContext;

/// A structured tag system for classifying states based on their purpose and behavior.
/// Tags are used to categorize states and to define dependencies between them.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Tag(pub &'static str);

/// Standard tags for classifying states by their kind
pub mod standard_tags {
    use super::Tag;

    // State kinds
    pub const CONFIG: Tag = Tag("config");
    pub const FETCH_DATA: Tag = Tag("fetch_data");
    pub const COMPUTE: Tag = Tag("compute");
    pub const EXECUTE: Tag = Tag("execute");
    pub const REPORT: Tag = Tag("report");
    pub const REPORT_OPERATOR: Tag = Tag("report_operator");
    pub const REPORT_OPERATION: Tag = Tag("report_operation");

    // State behavior
    pub const APPLY_SIDE_EFFECT: Tag = Tag("apply_side_effect");
    pub const IMPURE: Tag = Tag("impure");

    // Create a vec of all standard tags (useful for validation and documentation)
    pub fn all() -> Vec<Tag> {
        vec![
            CONFIG,
            FETCH_DATA,
            COMPUTE,
            EXECUTE,
            REPORT,
            REPORT_OPERATOR,
            REPORT_OPERATION,
            APPLY_SIDE_EFFECT,
            IMPURE,
        ]
    }

    // Create a vec of all state kind tags
    pub fn kind_tags() -> Vec<Tag> {
        vec![
            CONFIG,
            FETCH_DATA,
            COMPUTE,
            EXECUTE,
            REPORT,
            REPORT_OPERATOR,
            REPORT_OPERATION,
        ]
    }

    // Create a vec of all behavior tags
    pub fn behavior_tags() -> Vec<Tag> {
        vec![APPLY_SIDE_EFFECT, IMPURE]
    }
}

/// A unique identifier for a state
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Label(pub &'static str);

fn ensure_nonempty_ascii_lowercase_underscore(input: &'static str) -> Result<&'static str, Error> {
    if input.is_empty() {
        return Err(anyhow!("empty string; this string should be non empty, lowercase and use underscore as separator"));
    }

    if !input.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return Err(anyhow!("invalid char in '{}'; this string should be non empty, lowercase and use underscore as separator", input));
    }

    Ok(input)
}

impl Tag {
    pub fn new(s: &'static str) -> Result<Self, Error> {
        match ensure_nonempty_ascii_lowercase_underscore(s) {
            Ok(validated_input) => Ok(Self(validated_input)),
            Err(e) => Err(e),
        }
    }

    /// Creates a Tag without validation - use only for constant definitions
    /// where string validity is guaranteed at compile time
    pub const fn new_const(s: &'static str) -> Self {
        Self(s)
    }

    /// Returns the string value of the tag
    pub fn as_str(&self) -> &'static str {
        self.0
    }

    /// Checks if this tag is one of the standard state kind tags
    pub fn is_kind_tag(&self) -> bool {
        standard_tags::kind_tags().contains(self)
    }

    /// Checks if this tag is one of the standard behavior tags
    pub fn is_behavior_tag(&self) -> bool {
        standard_tags::behavior_tags().contains(self)
    }

    /// Checks if this tag is a standard tag
    pub fn is_standard_tag(&self) -> bool {
        standard_tags::all().contains(self)
    }
}

impl Label {
    pub fn new(s: &'static str) -> Result<Self, Error> {
        match ensure_nonempty_ascii_lowercase_underscore(s) {
            Ok(validated_input) => Ok(Self(validated_input)),
            Err(e) => Err(e),
        }
    }

    /// Creates a Label without validation - use only for constant definitions
    /// where string validity is guaranteed at compile time
    pub const fn new_const(s: &'static str) -> Self {
        Self(s)
    }

    /// Returns the string value of the label
    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

impl From<Label> for String {
    fn from(value: Label) -> Self {
        value.0.to_owned()
    }
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum DependencyStrategy {
    Latest,
    Earliest,
    LatestSuccessful,
}

/// Metadata for state handlers, providing information about state identity,
/// categorization via tags, and dependency information.
pub trait StateMetadata {
    /// Returns the unique label identifying this state
    fn label(&self) -> Label;

    /// Returns all tags associated with this state
    fn tags(&self) -> Vec<Tag>;

    /// Returns tags that this state depends on
    fn depends_on(&self) -> Vec<Tag>;

    /// Returns the strategy for resolving dependencies
    fn depends_on_strategy(&self) -> DependencyStrategy;

    /// Returns true if this state has a tag indicating it applies side effects
    fn has_side_effects(&self) -> bool {
        self.tags().contains(&standard_tags::APPLY_SIDE_EFFECT)
    }

    /// Returns true if this state has a tag indicating it is impure
    fn is_impure(&self) -> bool {
        self.tags().contains(&standard_tags::IMPURE)
    }

    /// Returns the kind tag of this state if present
    fn kind_tag(&self) -> Option<Tag> {
        self.tags().into_iter().find(|tag| tag.is_kind_tag())
    }
}

pub type StateResult = Result<(), StateError>;

/// A state handler that can be executed by the state machine.
/// It maintains metadata about itself and provides a handler function
/// to process the context data.
pub trait StateHandler: StateMetadata + Send + Sync {
    /// Executes this state's logic, potentially modifying the context
    fn handler(&self, context: SafeContext) -> StateResult;
}

pub type States = Arc<[Box<dyn StateHandler>]>;

#[derive(Debug, Clone)]
pub enum StateErrorRecoverability {
    Recoverable,
    Unrecoverable,
}

#[derive(Debug)]
pub enum StateError {
    Unknown(StateErrorRecoverability, anyhow::Error),
    ParsingInput(StateErrorRecoverability, anyhow::Error),
    OnChainError(StateErrorRecoverability, anyhow::Error),
    OffChainError(StateErrorRecoverability, anyhow::Error),
    RpcConnection(StateErrorRecoverability, anyhow::Error),
    StorageAccess(StateErrorRecoverability, anyhow::Error),
}

impl StateError {
    pub fn is_recoverable(&self) -> bool {
        match self {
            Self::Unknown(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::RpcConnection(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::StorageAccess(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::OnChainError(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::OffChainError(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
            Self::ParsingInput(recov, _) => matches!(recov, StateErrorRecoverability::Recoverable),
        }
    }

    /// Creates a new Unknown recoverable error
    pub fn recoverable_unknown(error: impl Into<anyhow::Error>) -> Self {
        Self::Unknown(StateErrorRecoverability::Recoverable, error.into())
    }

    /// Creates a new Unknown unrecoverable error
    pub fn unrecoverable_unknown(error: impl Into<anyhow::Error>) -> Self {
        Self::Unknown(StateErrorRecoverability::Unrecoverable, error.into())
    }

    /// Creates a new ParsingInput recoverable error
    pub fn recoverable_parsing_input(error: impl Into<anyhow::Error>) -> Self {
        Self::ParsingInput(StateErrorRecoverability::Recoverable, error.into())
    }

    /// Creates a new ParsingInput unrecoverable error
    pub fn unrecoverable_parsing_input(error: impl Into<anyhow::Error>) -> Self {
        Self::ParsingInput(StateErrorRecoverability::Unrecoverable, error.into())
    }

    /// Creates a new OnChainError recoverable error
    pub fn recoverable_on_chain(error: impl Into<anyhow::Error>) -> Self {
        Self::OnChainError(StateErrorRecoverability::Recoverable, error.into())
    }

    /// Creates a new OnChainError unrecoverable error
    pub fn unrecoverable_on_chain(error: impl Into<anyhow::Error>) -> Self {
        Self::OnChainError(StateErrorRecoverability::Unrecoverable, error.into())
    }

    /// Creates a new OffChainError recoverable error
    pub fn recoverable_off_chain(error: impl Into<anyhow::Error>) -> Self {
        Self::OffChainError(StateErrorRecoverability::Recoverable, error.into())
    }

    /// Creates a new OffChainError unrecoverable error
    pub fn unrecoverable_off_chain(error: impl Into<anyhow::Error>) -> Self {
        Self::OffChainError(StateErrorRecoverability::Unrecoverable, error.into())
    }

    /// Creates a new RpcConnection recoverable error
    pub fn recoverable_rpc_connection(error: impl Into<anyhow::Error>) -> Self {
        Self::RpcConnection(StateErrorRecoverability::Recoverable, error.into())
    }

    /// Creates a new RpcConnection unrecoverable error
    pub fn unrecoverable_rpc_connection(error: impl Into<anyhow::Error>) -> Self {
        Self::RpcConnection(StateErrorRecoverability::Unrecoverable, error.into())
    }

    /// Creates a new StorageAccess recoverable error
    pub fn recoverable_storage_access(error: impl Into<anyhow::Error>) -> Self {
        Self::StorageAccess(StateErrorRecoverability::Recoverable, error.into())
    }

    /// Creates a new StorageAccess unrecoverable error
    pub fn unrecoverable_storage_access(error: impl Into<anyhow::Error>) -> Self {
        Self::StorageAccess(StateErrorRecoverability::Unrecoverable, error.into())
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(r, e) => write!(
                f,
                "unknown error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
            Self::RpcConnection(r, e) => write!(
                f,
                "RPC connection error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
            Self::StorageAccess(r, e) => write!(
                f,
                "storage access error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
            Self::OnChainError(r, e) => write!(
                f,
                "on-chain error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
            Self::OffChainError(r, e) => write!(
                f,
                "off-chain error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
            Self::ParsingInput(r, e) => write!(
                f,
                "parsing input error; recoverability: {:?}; source error: {:?}",
                r, e
            ),
        }
    }
}

impl std::error::Error for StateError {}

// Implement StateHandler for Box<dyn StateHandler>
impl StateHandler for Box<dyn StateHandler> {
    fn handler(&self, context: SafeContext) -> StateResult {
        (**self).handler(context)
    }
}

// Implement StateMetadata for Box<dyn StateHandler>
impl StateMetadata for Box<dyn StateHandler> {
    fn label(&self) -> Label {
        (**self).label()
    }

    fn tags(&self) -> Vec<Tag> {
        (**self).tags()
    }

    fn depends_on(&self) -> Vec<Tag> {
        (**self).depends_on()
    }

    fn depends_on_strategy(&self) -> DependencyStrategy {
        (**self).depends_on_strategy()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn custom_error_to_anyhow_error() {
        let state_error_to_anyhow = |error: StateError| -> anyhow::Error { error.into() };
        state_error_to_anyhow(StateError::Unknown(
            StateErrorRecoverability::Unrecoverable,
            anyhow!("test error"),
        ));
    }

    #[test]
    fn test_valid_input_ensure_nonempty_ascii_lowercase_underscore() {
        ["this_should_work", "this_should_work_also", "thisalso"]
            .iter()
            .for_each(|input| {
                let result = ensure_nonempty_ascii_lowercase_underscore(input);
                assert!(result.is_ok());
                assert_eq!(&result.unwrap(), input);
            })
    }

    #[test]
    fn test_invalid_spaces_input_ensure_nonempty_ascii_lowercase_underscore() {
        let s = "this shouldnt work";
        let result = ensure_nonempty_ascii_lowercase_underscore(s);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_input_ensure_nonempty_ascii_lowercase_underscore() {
        let s = "";
        let result = ensure_nonempty_ascii_lowercase_underscore(s);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_special_char_input_ensure_nonempty_ascii_lowercase_underscore() {
        let s = "this_should_not_work_@";
        let result = ensure_nonempty_ascii_lowercase_underscore(s);
        assert!(result.is_err());
    }

    #[test]
    fn test_standard_tags() {
        // Test kind tag detection
        assert!(standard_tags::CONFIG.is_kind_tag());
        assert!(standard_tags::REPORT.is_kind_tag());
        assert!(!standard_tags::APPLY_SIDE_EFFECT.is_kind_tag());

        // Test behavior tag detection
        assert!(standard_tags::APPLY_SIDE_EFFECT.is_behavior_tag());
        assert!(standard_tags::IMPURE.is_behavior_tag());
        assert!(!standard_tags::CONFIG.is_behavior_tag());

        // Test standard tag detection
        assert!(standard_tags::CONFIG.is_standard_tag());
        assert!(standard_tags::APPLY_SIDE_EFFECT.is_standard_tag());

        // Test a non-standard tag
        let custom_tag = Tag::new("custom_tag").unwrap();
        assert!(!custom_tag.is_standard_tag());
        assert!(!custom_tag.is_kind_tag());
        assert!(!custom_tag.is_behavior_tag());
    }
}
