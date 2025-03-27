use anyhow::{anyhow, Result};
use std::sync::Arc;

use crate::state::{
    safe_context::SafeContext, DependencyStrategy, Label, StateError, StateHandler, StateMetadata,
    StateResult, States, Tag,
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
    fn next_state_safe(
        &self,
        current_index: usize,
        states: &States,
        context: &SafeContext,
    ) -> Result<usize, SchedulerError>;

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
        tracker: &Box<dyn Tracker>,
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
    fn next_state_safe(
        &self,
        current_index: usize,
        states: &States,
        _context: &SafeContext,
    ) -> Result<usize, SchedulerError> {
        if states.is_empty() {
            return Err(SchedulerError::NoStates);
        }

        let next_index = current_index + 1;

        if next_index >= states.len() {
            // We've reached the end of the states
            // Return NoNextState to indicate completion
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

/// A default implementation of the ErrorHandler trait
/// This handler follows dependencies to determine recovery paths
pub struct DefaultErrorHandler {}

impl DefaultErrorHandler {
    /// Create a new DefaultErrorHandler
    pub fn new() -> Self {
        Self {}
    }
}

impl ErrorHandler for DefaultErrorHandler {
    fn handle_error(
        &self,
        error: &StateError,
        current_index: usize,
        current_state: &dyn StateHandler,
        states: &States,
        tracker: &Box<dyn Tracker>,
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

        // Find the last successful execution of a state with the first dependency tag
        let first_dependency = depends_on.first().unwrap();
        let dependency_indexes = tracker.search_by_tag(first_dependency);

        if dependency_indexes.is_empty() {
            // If no dependency state has been executed yet, restart from the beginning
            return Ok(0);
        }

        // Use the last executed dependency state as the recovery point
        let last_dependency = dependency_indexes.last().unwrap();

        Ok(last_dependency.state_index)
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
    fn next_state_safe(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        safe_context::create_default_safe_context, DependencyStrategy, Label, StateHandler,
        StateResult,
    };
    use mfm_machine_derive::StateMetadataReqs;

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
                label: Label::new_const("test_state_1"),
                tags: vec![Tag::new_const("test1")],
                depends_on: vec![],
                depends_on_strategy: DependencyStrategy::Latest,
            }
        }
    }

    impl StateHandler for TestState1 {
        fn handler(&self, _context: SafeContext) -> StateResult {
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
                label: Label::new_const("test_state_2"),
                tags: vec![Tag::new_const("test2")],
                depends_on: vec![Tag::new_const("test1")],
                depends_on_strategy: DependencyStrategy::Latest,
            }
        }
    }

    impl StateHandler for TestState2 {
        fn handler(&self, _context: SafeContext) -> StateResult {
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
        let next_index = scheduler
            .next_state_safe(0, &states, &safe_context)
            .unwrap();
        assert_eq!(next_index, 1);

        // Test error at end of states
        let result = scheduler.next_state_safe(1, &states, &safe_context);
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
        let next_index = scheduler
            .next_state_safe(0, &states, &safe_context)
            .unwrap();
        assert_eq!(next_index, 1);
    }
}
