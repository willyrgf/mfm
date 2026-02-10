use anyhow::{anyhow, Result};
use std::collections::HashMap;

use crate::state::{
    safe_context::SafeContext, StateError, StateHandler, StateMetadata, States, Tag,
};

use super::tracker::Tracker;

/// Errors that can occur in the scheduler
#[derive(Debug)]
pub enum SchedulerError {
    /// No next state available
    NoNextState,
    /// Invalid state index
    InvalidStateIndex(usize),
    /// No states available
    NoStates,
    /// Custom scheduler error
    Custom(anyhow::Error),
}

/// A scheduler that determines the next state to execute based on the current state and context
pub trait Scheduler: Send + Sync {
    /// Determine the next state to execute using SafeContext
    fn next_state(
        &self,
        current_index: usize,
        states: &States,
        context: &SafeContext,
    ) -> Result<usize, SchedulerError>;

    /// Determine the next state(s) to execute using SafeContext.
    ///
    /// Default behavior is single-step scheduling (sequential).
    fn next_states(
        &self,
        current_index: usize,
        states: &States,
        context: &SafeContext,
    ) -> Result<Vec<usize>, SchedulerError> {
        Ok(vec![self.next_state(current_index, states, context)?])
    }

    /// Set tags to filter out states during execution
    fn set_filter_tags(&mut self, tags: Option<Vec<Tag>>);

    /// Get the current filter tags
    fn get_filter_tags(&self) -> Option<Vec<Tag>>;
}

/// An error handler that determines how to recover from errors
pub trait ErrorHandler: Send + Sync {
    /// Handle an error and determine whether to continue execution
    /// Returns the index of the state to continue from, or an error if execution should stop
    fn handle_error(
        &self,
        error: &StateError,
        current_index: usize,
        current_state: &dyn StateHandler,
        states: &States,
        tracker: &dyn Tracker,
    ) -> Result<usize, SchedulerError>;
}

/// A default implementation of the Scheduler trait
/// This scheduler simply executes states in sequence
pub struct DefaultScheduler {
    filter_tags: Option<Vec<Tag>>,
}

impl DefaultScheduler {
    /// Create a new DefaultScheduler
    pub fn new() -> Self {
        Self { filter_tags: None }
    }
}

impl Scheduler for DefaultScheduler {
    fn next_state(
        &self,
        current_index: usize,
        states: &States,
        _context: &SafeContext,
    ) -> Result<usize, SchedulerError> {
        if states.is_empty() {
            return Err(SchedulerError::NoStates);
        }

        if current_index >= states.len() {
            return Err(SchedulerError::InvalidStateIndex(current_index));
        }

        // Just increment to the next state
        let next_index = current_index + 1;
        if next_index >= states.len() {
            return Err(SchedulerError::NoNextState);
        }

        Ok(next_index)
    }

    fn set_filter_tags(&mut self, tags: Option<Vec<Tag>>) {
        self.filter_tags = tags;
    }

    fn get_filter_tags(&self) -> Option<Vec<Tag>> {
        self.filter_tags.clone()
    }
}

impl Default for DefaultScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Default implementation of the ErrorHandler trait
pub struct DefaultErrorHandler;

impl DefaultErrorHandler {
    /// Create a new DefaultErrorHandler
    pub fn new() -> Self {
        Self
    }
}

impl ErrorHandler for DefaultErrorHandler {
    fn handle_error(
        &self,
        error: &StateError,
        _current_index: usize,
        current_state: &dyn StateHandler,
        _states: &States,
        tracker: &dyn Tracker,
    ) -> Result<usize, SchedulerError> {
        if !error.is_recoverable() {
            return Err(SchedulerError::Custom(anyhow!("Unrecoverable error")));
        }

        // Try to find a suitable recovery point based on dependencies
        let depends_on = current_state.depends_on();

        if depends_on.is_empty() {
            // If there are no dependencies, restart from the beginning
            return Ok(0);
        }

        // Collect all candidate recovery points for all dependency tags.
        let mut dependency_indexes = Vec::new();
        for dependency_tag in &depends_on {
            dependency_indexes.extend(tracker.search_by_tag(dependency_tag));
        }

        if dependency_indexes.is_empty() {
            // If no dependency state has been executed yet, restart from the beginning.
            return Ok(0);
        }

        // Apply the dependency strategy to select a recovery point.
        let strategy = current_state.depends_on_strategy();
        let selected = match strategy {
            crate::state::DependencyStrategy::Latest => dependency_indexes
                .iter()
                .max_by_key(|idx| idx.state_index)
                .expect("dependency_indexes is non-empty"),
            crate::state::DependencyStrategy::Earliest => dependency_indexes
                .iter()
                .min_by_key(|idx| idx.state_index)
                .expect("dependency_indexes is non-empty"),
            crate::state::DependencyStrategy::LatestSuccessful => {
                // TODO: tracker does not record success/failure per execution yet.
                dependency_indexes
                    .iter()
                    .max_by_key(|idx| idx.state_index)
                    .expect("dependency_indexes is non-empty")
            }
        };

        Ok(selected.state_index)
    }
}

impl Default for DefaultErrorHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// A scheduler that follows a dependency graph to determine the next state
pub struct DependencyScheduler {
    filter_tags: Option<Vec<Tag>>,
}

impl DependencyScheduler {
    /// Create a new DependencyScheduler
    pub fn new() -> Self {
        Self { filter_tags: None }
    }

    /// Find the next state based on dependency relationships
    fn find_next_state_by_dependencies(
        &self,
        current_state: &dyn StateHandler,
        states: &States,
    ) -> Result<usize, SchedulerError> {
        let current_tags = current_state.tags();

        // Find states that depend on the current state's tags
        for (index, state) in states.iter().enumerate() {
            let depends_on = state.depends_on();

            // Check if this state depends on any of the current state's tags
            if depends_on.iter().any(|tag| current_tags.contains(tag)) {
                return Ok(index);
            }
        }

        // If no dependency relationship is found, use the next sequential state
        Err(SchedulerError::NoNextState)
    }
}

impl Scheduler for DependencyScheduler {
    fn next_state(
        &self,
        current_index: usize,
        states: &States,
        _context: &SafeContext,
    ) -> Result<usize, SchedulerError> {
        if states.is_empty() {
            return Err(SchedulerError::NoStates);
        }

        if current_index >= states.len() {
            return Err(SchedulerError::InvalidStateIndex(current_index));
        }

        let current_state = &states[current_index];

        // Try to find next state based on dependencies
        match self.find_next_state_by_dependencies(current_state, states) {
            Ok(next_index) => Ok(next_index),
            Err(_) => {
                // Fall back to sequential execution
                let next_index = current_index + 1;
                if next_index >= states.len() {
                    return Err(SchedulerError::NoNextState);
                }
                Ok(next_index)
            }
        }
    }

    fn set_filter_tags(&mut self, tags: Option<Vec<Tag>>) {
        self.filter_tags = tags;
    }

    fn get_filter_tags(&self) -> Option<Vec<Tag>> {
        self.filter_tags.clone()
    }
}

impl Default for DependencyScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// A scheduler that can return multiple independent next states for concurrent execution.
///
/// This implementation assumes the `states` slice is in dependency order, and will return a
/// contiguous block of runnable states starting at `current_index + 1` whose dependencies are all
/// satisfied by tags provided by states at indexes `<= current_index`.
pub struct ParallelScheduler {
    filter_tags: Option<Vec<Tag>>,
}

impl ParallelScheduler {
    pub fn new() -> Self {
        Self { filter_tags: None }
    }

    fn earliest_tag_providers(states: &States) -> HashMap<Tag, usize> {
        let mut providers = HashMap::new();
        for (idx, state) in states.iter().enumerate() {
            for tag in state.tags() {
                providers
                    .entry(tag)
                    .and_modify(|existing| {
                        if idx < *existing {
                            *existing = idx;
                        }
                    })
                    .or_insert(idx);
            }
        }
        providers
    }
}

impl Scheduler for ParallelScheduler {
    fn next_state(
        &self,
        current_index: usize,
        states: &States,
        context: &SafeContext,
    ) -> Result<usize, SchedulerError> {
        // Sequential fallback.
        DefaultScheduler::new().next_state(current_index, states, context)
    }

    fn next_states(
        &self,
        current_index: usize,
        states: &States,
        _context: &SafeContext,
    ) -> Result<Vec<usize>, SchedulerError> {
        if states.is_empty() {
            return Err(SchedulerError::NoStates);
        }

        if current_index >= states.len() {
            return Err(SchedulerError::InvalidStateIndex(current_index));
        }

        let start = current_index + 1;
        if start >= states.len() {
            return Err(SchedulerError::NoNextState);
        }

        let providers = Self::earliest_tag_providers(states);
        let mut out = Vec::new();

        for idx in start..states.len() {
            let state = &states[idx];
            let deps = state.depends_on();
            let runnable = deps.iter().all(|dep| {
                providers
                    .get(dep)
                    .is_some_and(|&provider_idx| provider_idx <= current_index)
            });

            if runnable {
                out.push(idx);
            } else {
                break;
            }
        }

        if out.is_empty() {
            return Err(SchedulerError::Custom(anyhow!(
                "No runnable next state at index {start}; state ordering likely violates dependencies"
            )));
        }

        Ok(out)
    }

    fn set_filter_tags(&mut self, tags: Option<Vec<Tag>>) {
        self.filter_tags = tags;
    }

    fn get_filter_tags(&self) -> Option<Vec<Tag>> {
        self.filter_tags.clone()
    }
}

impl Default for ParallelScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::state_machine::StateResult;
    use std::sync::Arc;

    use super::*;
    use crate::state::{
        safe_context::create_default_safe_context, DependencyStrategy, Label, StateHandler,
    };
    use mfm_machine_derive_legacy::StateMetadataReqs;

    // Define test states
    #[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
    struct TestState1 {
        label: Label,
        tags: Vec<Tag>,
        depends_on: Vec<Tag>,
        depends_on_strategy: DependencyStrategy,
    }

    impl TestState1 {
        fn new() -> Self {
            Self {
                label: Label::new_unchecked("test_state_1"),
                tags: vec![Tag::new_unchecked("test1")],
                depends_on: vec![],
                depends_on_strategy: DependencyStrategy::Latest,
            }
        }
    }

    #[async_trait::async_trait]
    impl StateHandler for TestState1 {
        async fn handler(&self, _context: SafeContext) -> StateResult {
            // State 1 always passes
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
    struct TestState2 {
        label: Label,
        tags: Vec<Tag>,
        depends_on: Vec<Tag>,
        depends_on_strategy: DependencyStrategy,
    }

    impl TestState2 {
        fn new() -> Self {
            Self {
                label: Label::new_unchecked("test_state_2"),
                tags: vec![Tag::new_unchecked("test2")],
                depends_on: vec![Tag::new_unchecked("test1")],
                depends_on_strategy: DependencyStrategy::Latest,
            }
        }
    }

    #[async_trait::async_trait]
    impl StateHandler for TestState2 {
        async fn handler(&self, _context: SafeContext) -> StateResult {
            // State 2 always passes
            Ok(())
        }
    }

    #[test]
    fn test_default_scheduler() {
        let scheduler = DefaultScheduler::new();
        let safe_context = create_default_safe_context();

        let states: States = Arc::from(vec![
            Box::new(TestState1::new()) as Box<dyn StateHandler>,
            Box::new(TestState2::new()) as Box<dyn StateHandler>,
        ]);

        // Test next state calculation
        let next_index = scheduler.next_state(0, &states, &safe_context).unwrap();
        assert_eq!(next_index, 1);

        // Test error at end of states
        let result = scheduler.next_state(1, &states, &safe_context);
        assert!(matches!(result, Err(SchedulerError::NoNextState)));
    }

    #[test]
    fn test_dependency_scheduler() {
        let scheduler = DependencyScheduler::new();
        let safe_context = create_default_safe_context();

        let states: States = Arc::from(vec![
            Box::new(TestState1::new()) as Box<dyn StateHandler>,
            Box::new(TestState2::new()) as Box<dyn StateHandler>,
        ]);

        // Test next state calculation
        let next_index = scheduler.next_state(0, &states, &safe_context).unwrap();
        assert_eq!(next_index, 1);
    }
}
