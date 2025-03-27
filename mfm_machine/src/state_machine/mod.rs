use anyhow::{anyhow, Result};

use crate::state::{safe_context::SafeContext, StateError, StateHandler, StateResult, States};

use self::scheduler::{
    DefaultErrorHandler, DefaultScheduler, ErrorHandler, Scheduler, SchedulerError,
};
use self::tracker::{HashMapTracker, Index, Tracker, TrackerHistory};

pub mod scheduler;
pub mod tracker;

/// Builder pattern for constructing a StateMachine with various configurations
pub struct StateMachineBuilder {
    pub states: States,
    pub tracker: Option<Box<dyn Tracker>>,
    pub scheduler: Option<Box<dyn Scheduler>>,
    pub error_handler: Option<Box<dyn ErrorHandler>>,
    pub max_recoveries: usize,
}

pub const MAX_RECOVERIES_MULT: usize = 3;

/// Default maximum number of recovery attempts allowed
/// Calculated as number of states * MAX_RECOVERIES_MULT + 1
pub fn default_max_recoveries(states: States) -> usize {
    states.len() * MAX_RECOVERIES_MULT + 1
}

impl StateMachineBuilder {
    /// Create a new StateMachineBuilder with the provided states
    pub fn new(states: States) -> Self {
        Self {
            states: states.clone(),
            tracker: None,
            scheduler: None,
            error_handler: None,
            max_recoveries: default_max_recoveries(states),
        }
    }

    /// Set a custom tracker for the state machine
    pub fn tracker(mut self, tracker: Box<dyn Tracker>) -> Self {
        self.tracker = Some(tracker);
        self
    }

    /// Set a custom scheduler for the state machine
    pub fn scheduler(mut self, scheduler: Box<dyn Scheduler>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    /// Set a custom error handler for the state machine
    pub fn error_handler(mut self, error_handler: Box<dyn ErrorHandler>) -> Self {
        self.error_handler = Some(error_handler);
        self
    }

    /// Set the maximum number of recovery attempts
    pub fn max_recoveries(mut self, max: usize) -> Self {
        self.max_recoveries = max;
        self
    }

    /// Build the StateMachine with the configured options
    pub fn build(self) -> StateMachine {
        StateMachine {
            states: self.states,
            tracker: self
                .tracker
                .unwrap_or_else(|| Box::new(HashMapTracker::new())),
            scheduler: self
                .scheduler
                .unwrap_or_else(|| Box::new(DefaultScheduler::new())),
            error_handler: self
                .error_handler
                .unwrap_or_else(|| Box::new(DefaultErrorHandler::new())),
            max_recoveries: self.max_recoveries,
        }
    }
}

/// The state machine that executes a sequence of states
pub struct StateMachine {
    pub states: States,
    pub tracker: Box<dyn Tracker>,
    pub scheduler: Box<dyn Scheduler>,
    pub error_handler: Box<dyn ErrorHandler>,
    max_recoveries: usize,
}

/// Errors that can occur during state machine execution
#[derive(Debug)]
pub enum StateMachineError {
    /// Maximum number of recovery attempts has been reached
    ReachedMaxRecoveries(usize, anyhow::Error),
    /// No states were provided to execute
    EmptyState(anyhow::Error),
    /// An internal error occurred in the state machine
    InternalError(StateResult, anyhow::Error),
    /// An error occurred in a state handler
    StateError(StateResult, anyhow::Error),
    /// An error occurred in the scheduler
    SchedulerError(SchedulerError),
    /// Custom error handler rejected continuing execution
    ErrorHandlerRejectedContinuation(StateError),
    /// Invalid state index
    InvalidStateIndex(usize),
    /// Invalid recovery state index
    InvalidRecoveryStateIndex(usize),
    /// Recovery error
    RecoveryError(StateResult, SchedulerError),
}

impl From<SchedulerError> for StateMachineError {
    fn from(error: SchedulerError) -> Self {
        StateMachineError::SchedulerError(error)
    }
}

impl StateMachine {
    /// Create a new StateMachine with default tracker, scheduler, and error handler
    pub fn new(states: States) -> Self {
        Self {
            states: states.clone(),
            tracker: Box::new(HashMapTracker::new()),
            scheduler: Box::new(DefaultScheduler::new()),
            error_handler: Box::new(DefaultErrorHandler::new()),
            max_recoveries: default_max_recoveries(states),
        }
    }

    /// Get the history of executed states
    pub fn track_history(&self) -> TrackerHistory {
        self.tracker.history()
    }

    /// Check if the state machine has a state at the given index
    fn has_state(&self, state_index: usize) -> bool {
        self.states.len() > state_index
    }

    /// Check if the maximum number of recovery attempts has been reached
    fn reached_max_recoveries(&self) -> (bool, usize) {
        let steps = self.track_history().len();
        (steps >= self.max_recoveries, steps)
    }

    /// Determine the next state and context based on the result of the previous state
    fn transition_safe(
        &mut self,
        context: SafeContext,
        state_index: usize,
        last_state_result: Option<StateResult>,
    ) -> Result<(usize, SafeContext), StateMachineError> {
        // Check if we have any states
        if !self.has_state(0) {
            return Err(StateMachineError::EmptyState(anyhow!(
                "There are no states to execute"
            )));
        }

        // Check if we've exceeded the maximum number of recovery attempts
        let (reached_max, steps) = self.reached_max_recoveries();
        if reached_max {
            return Err(StateMachineError::ReachedMaxRecoveries(
                steps,
                anyhow!("Reached max recoveries ({})", steps),
            ));
        }

        // If this is the first state execution
        if last_state_result.is_none() {
            return Ok((state_index, context));
        }

        let state_result = last_state_result.unwrap();
        let state = if self.has_state(state_index) {
            &self.states[state_index]
        } else {
            return Err(StateMachineError::InvalidStateIndex(state_index));
        };

        match state_result {
            Ok(()) => {
                // Successful execution - determine next state via scheduler
                let next_index =
                    self.scheduler
                        .next_state_safe(state_index, &self.states, &context)?;
                Ok((next_index, context))
            }
            Err(e) => {
                // Handle error based on recoverability
                if e.is_recoverable() {
                    // Use error handler to determine recovery strategy
                    let recovery_result = self.error_handler.handle_error(
                        &e,
                        state_index,
                        state,
                        &self.states,
                        &self.tracker,
                    );

                    match recovery_result {
                        Ok(recovery_index) => {
                            // Get the context from the recovery point
                            if let Some(recovery_state_index) =
                                self.tracker.search_by_index(&recovery_index)
                            {
                                if let Some(recovery_context) =
                                    self.tracker.recover(recovery_state_index.clone())
                                {
                                    return Ok((recovery_index, recovery_context));
                                }
                            }

                            // If we can't get context from the exact recovery point, try finding by tag
                            let state_depends_on = state.depends_on();
                            if !state_depends_on.is_empty() {
                                let indexes_state_deps = self
                                    .tracker
                                    .search_by_tag(state_depends_on.first().unwrap());

                                if let Some(last_index) = indexes_state_deps.last() {
                                    if let Some(recovery_context) =
                                        self.tracker.recover(last_index.clone())
                                    {
                                        return Ok((last_index.state_index, recovery_context));
                                    }
                                }
                            }

                            // If all else fails, restart from beginning with original context
                            Ok((0, context))
                        }
                        Err(_) => {
                            // Error handler rejected continuing
                            Err(StateMachineError::ErrorHandlerRejectedContinuation(e))
                        }
                    }
                } else {
                    // Unrecoverable error
                    Err(StateMachineError::StateError(
                        Err(e),
                        anyhow!("An unrecoverable error occurred in a state handler"),
                    ))
                }
            }
        }
    }

    /// Recursively execute states using SafeContext
    fn execute_rec_safe(
        &mut self,
        context: SafeContext,
        state_index: usize,
        last_state_result: Option<StateResult>,
    ) -> Result<SafeContext, StateMachineError> {
        let transition_result =
            self.transition_safe(context.clone(), state_index, last_state_result);

        match transition_result {
            Ok((next_state_index, context)) => {
                // If there are no more states to execute, return the final context
                if !self.has_state(next_state_index) {
                    return Ok(context);
                }

                let state = &self.states[next_state_index];

                // Check if we should skip this state based on filtering conditions
                if let Some(filter_tags) = self.scheduler.get_filter_tags() {
                    let state_tags = state.tags();
                    if filter_tags
                        .iter()
                        .any(|filter_tag| state_tags.contains(filter_tag))
                    {
                        println!(
                            "Skipping state at index {} because of filter",
                            next_state_index
                        );
                        // Skip this state and move to the next one
                        // First we need to determine what the next state is
                        let next_next_index = match self.scheduler.next_state_safe(
                            next_state_index,
                            &self.states,
                            &context,
                        ) {
                            Ok(idx) => idx,
                            Err(SchedulerError::NoNextState) => {
                                // No more states - we're done
                                return Ok(context);
                            }
                            Err(err) => return Err(StateMachineError::SchedulerError(err)),
                        };

                        return self.execute_rec_safe(context, next_next_index, Some(Ok(())));
                    }
                }

                // Execute the state handler
                let result = state.handler_safe(context.clone());

                // Track the execution
                let _ = self.tracker.as_mut().track(
                    Index::new(next_state_index, state.label(), state.tags()),
                    context.clone(),
                );

                // Continue to the next state
                self.execute_rec_safe(context, next_state_index, Some(result))
            }
            Err(StateMachineError::SchedulerError(SchedulerError::NoNextState)) => {
                // We've reached the end of states - this is a successful completion
                Ok(context)
            }
            Err(err) => Err(err),
        }
    }

    /// Execute the state machine with the given SafeContext
    pub fn execute_safe(&mut self, context: SafeContext) -> Result<SafeContext, StateMachineError> {
        self.execute_rec_safe(context, 0, None)
    }

    /// Execute the state machine starting from a specific state index
    pub fn execute_from_safe(
        &mut self,
        context: SafeContext,
        start_index: usize,
    ) -> Result<SafeContext, StateMachineError> {
        if !self.has_state(start_index) {
            return Err(StateMachineError::EmptyState(anyhow!(
                "Invalid start index: {}",
                start_index
            )));
        }

        self.execute_rec_safe(context, start_index, None)
    }

    /// Execute the state machine, filtering out states with specific tags, using SafeContext
    pub fn execute_with_filter_safe(
        &mut self,
        context: SafeContext,
        filter_tags: Vec<crate::state::Tag>,
    ) -> Result<SafeContext, StateMachineError> {
        self.scheduler.set_filter_tags(Some(filter_tags));
        let result = self.execute_safe(context);
        self.scheduler.set_filter_tags(None);
        result
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::state::safe_context::{create_default_safe_context, SafeContext};
    use crate::state::{
        standard_tags, DependencyStrategy, Label, StateError, StateErrorRecoverability,
        StateHandler, StateMetadata, StateResult, Tag,
    };
    use mfm_machine_derive::StateMetadataReqs;
    use serde_derive::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
    pub struct Setup {
        label: Label,
        tags: Vec<Tag>,
        depends_on: Vec<Tag>,
        depends_on_strategy: DependencyStrategy,
    }

    impl Setup {
        fn new() -> Self {
            Self {
                label: Label::new("setup_state").unwrap(),
                tags: vec![Tag::new("setup").unwrap(), standard_tags::CONFIG],
                depends_on: vec![Tag::new("setup").unwrap()],
                depends_on_strategy: DependencyStrategy::Latest,
            }
        }
    }

    #[derive(Serialize, Deserialize)]
    struct SetupCtx {
        a: String,
        b: u32,
    }

    impl StateHandler for Setup {
        fn handler(&self, context: SafeContext) -> StateResult {
            let data = SetupCtx {
                a: "setup_b".to_string(),
                b: 1,
            };

            context
                .write_typed("setup", &data)
                .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))
        }
    }

    #[derive(Serialize, Deserialize, Debug)]
    pub struct ReportCtx {
        a: String,
        c: u32,
    }

    #[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
    pub struct Report {
        label: Label,
        tags: Vec<Tag>,
        depends_on: Vec<Tag>,
        depends_on_strategy: DependencyStrategy,
    }

    impl Report {
        fn new() -> Self {
            Self {
                label: Label::new("report_state").unwrap(),
                tags: vec![Tag::new("report").unwrap(), standard_tags::REPORT],
                depends_on: vec![Tag::new("setup").unwrap()],
                depends_on_strategy: DependencyStrategy::Latest,
            }
        }
    }

    impl StateHandler for Report {
        fn handler(&self, context: SafeContext) -> StateResult {
            let setup_data: SetupCtx = context
                .read_typed("setup")
                .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))?;

            let report_data = ReportCtx {
                a: format!("{}_handled", setup_data.a),
                c: setup_data.b * 2,
            };

            context
                .write_typed("report", &report_data)
                .map_err(|e| StateError::StorageAccess(StateErrorRecoverability::Recoverable, e))
        }
    }

    #[test]
    fn test_state_machine_simple_flow() {
        let setup = Setup::new();
        let report = Report::new();

        let states: Vec<Box<dyn StateHandler>> = vec![
            Box::new(setup) as Box<dyn StateHandler>,
            Box::new(report) as Box<dyn StateHandler>,
        ];
        let states = Arc::from(states);

        println!("Creating state machine...");
        let mut state_machine = super::StateMachine::new(states);

        println!("Creating context...");
        let safe_context = create_default_safe_context();

        println!("Executing state machine...");
        let result = state_machine.execute_safe(safe_context);

        println!("Result: {:?}", result);
        assert!(result.is_ok());

        let safe_result = result.unwrap();
        println!("Reading report data...");
        let report_data: ReportCtx = safe_result.read_typed("report").unwrap();
        println!("Report data: {:?}", report_data);
        assert_eq!(report_data.a, "setup_b_handled");
        assert_eq!(report_data.c, 2);
    }

    #[test]
    fn test_state_machine_with_filter() {
        let setup = Setup::new();
        let report = Report::new();

        let states: Vec<Box<dyn StateHandler>> = vec![
            Box::new(setup) as Box<dyn StateHandler>,
            Box::new(report) as Box<dyn StateHandler>,
        ];
        let states = Arc::from(states);

        println!("Creating state machine...");
        let mut state_machine = super::StateMachine::new(states);

        println!("Creating context...");
        let safe_context = create_default_safe_context();

        println!("Setting filter tags: {:?}", standard_tags::REPORT);
        // Use the safe version with filter
        let result =
            state_machine.execute_with_filter_safe(safe_context, vec![standard_tags::REPORT]);

        println!("Result: {:?}", result);
        assert!(result.is_ok());

        // Report state should have been skipped, so no "report" data
        let final_context = result.unwrap();
        println!("Context keys: {:?}", final_context.dump().unwrap());
        assert!(final_context.read_typed::<SetupCtx>("setup").is_ok());
        println!("Setup context is present");
        assert!(final_context.read_typed::<ReportCtx>("report").is_err());
        println!("Report context is absent as expected");
    }
}
