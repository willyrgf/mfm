use anyhow::{anyhow, Result};
use std::collections::{HashSet, VecDeque};

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
    pub fn build(self) -> Result<StateMachine, StateMachineError> {
        validate_states(&self.states)?;

        Ok(StateMachine {
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
        })
    }
}

fn validate_states(states: &States) -> Result<(), StateMachineError> {
    if states.is_empty() {
        return Err(StateMachineError::EmptyState(anyhow!(
            "There are no states to execute"
        )));
    }

    // a) No duplicate labels.
    let mut labels = HashSet::new();
    for state in states.iter() {
        let label = state.label();
        if !labels.insert(label.clone()) {
            return Err(StateMachineError::DuplicateLabel(label));
        }
    }

    // b) All dependency tags are provided by at least one state.
    let mut all_tags = HashSet::new();
    for state in states.iter() {
        for tag in state.tags() {
            all_tags.insert(tag);
        }
    }
    for state in states.iter() {
        let label = state.label();
        for dep in state.depends_on() {
            if !all_tags.contains(&dep) {
                return Err(StateMachineError::DanglingDependency {
                    state_label: label,
                    missing_tag: dep,
                });
            }
        }
    }

    // c) No circular dependencies between states (topological sort).
    let n = states.len();
    let state_labels: Vec<crate::state::Label> = states.iter().map(|s| s.label()).collect();
    let state_tags: Vec<Vec<crate::state::Tag>> = states.iter().map(|s| s.tags()).collect();
    let state_depends_on: Vec<Vec<crate::state::Tag>> =
        states.iter().map(|s| s.depends_on()).collect();

    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_degree: Vec<usize> = vec![0; n];

    for i in 0..n {
        // Add edges from any state that provides a dependency tag to the dependent state.
        let mut predecessors = HashSet::new();
        for dep_tag in &state_depends_on[i] {
            for (j, tags) in state_tags.iter().enumerate() {
                if tags.contains(dep_tag) {
                    predecessors.insert(j);
                }
            }
        }

        for pred in predecessors {
            adjacency[pred].push(i);
            in_degree[i] += 1;
        }
    }

    let mut queue: VecDeque<usize> = in_degree
        .iter()
        .enumerate()
        .filter_map(|(i, &deg)| if deg == 0 { Some(i) } else { None })
        .collect();

    let mut visited = 0usize;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        for &succ in &adjacency[node] {
            // Each edge is counted once in `in_degree`, so this cannot underflow.
            in_degree[succ] -= 1;
            if in_degree[succ] == 0 {
                queue.push_back(succ);
            }
        }
    }

    if visited != n {
        let cycle = in_degree
            .iter()
            .enumerate()
            .filter_map(|(i, &deg)| {
                if deg > 0 {
                    Some(state_labels[i].clone())
                } else {
                    None
                }
            })
            .collect();
        return Err(StateMachineError::CircularDependency(cycle));
    }

    Ok(())
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
    /// Duplicate state label found
    DuplicateLabel(crate::state::Label),
    /// A state depends on a tag that no state provides
    DanglingDependency {
        state_label: crate::state::Label,
        missing_tag: crate::state::Tag,
    },
    /// Circular dependency between states
    CircularDependency(Vec<crate::state::Label>),
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
    pub fn new(states: States) -> Result<Self, StateMachineError> {
        StateMachineBuilder::new(states).build()
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
    fn transition(
        &mut self,
        context: SafeContext,
        state_index: usize,
        last_state_result: Option<StateResult>,
    ) -> Result<(Vec<usize>, SafeContext), StateMachineError> {
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
            return Ok((vec![state_index], context));
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
                let next_indexes = if self.scheduler.get_filter_tags().is_some() {
                    vec![self
                        .scheduler
                        .next_state(state_index, &self.states, &context)?]
                } else {
                    self.scheduler
                        .next_states(state_index, &self.states, &context)?
                };

                Ok((next_indexes, context))
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
                                    return Ok((vec![recovery_index], recovery_context));
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
                                        return Ok((
                                            vec![last_index.state_index],
                                            recovery_context,
                                        ));
                                    }
                                }
                            }

                            // TODO: human: think if we want this or just throw error (config?)
                            // If all else fails, restart from beginning with original context
                            Ok((vec![0], context))
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

    /// Recursively execute states
    async fn execute_rec(
        &mut self,
        mut context: SafeContext,
        mut state_index: usize,
        mut last_state_result: Option<StateResult>,
    ) -> Result<SafeContext, StateMachineError> {
        loop {
            let transition_result =
                self.transition(context.clone(), state_index, last_state_result);

            match transition_result {
                Ok((next_state_indexes, next_context)) => {
                    context = next_context;

                    // If there are no more states to execute, return the final context.
                    if next_state_indexes
                        .iter()
                        .all(|&next_state_index| !self.has_state(next_state_index))
                    {
                        return Ok(context);
                    }

                    // If filtering is enabled, keep execution sequential to preserve existing
                    // skip semantics.
                    let filter_tags = self.scheduler.get_filter_tags();
                    if filter_tags.is_some() || next_state_indexes.len() <= 1 {
                        let next_state_index = next_state_indexes
                            .into_iter()
                            .find(|&i| self.has_state(i))
                            .expect("at least one next index is in range");

                        let state = &self.states[next_state_index];

                        // Check if we should skip this state based on filtering conditions.
                        if let Some(filter_tags) = filter_tags {
                            let state_tags = state.tags();
                            if filter_tags
                                .iter()
                                .any(|filter_tag| state_tags.contains(filter_tag))
                            {
                                log::debug!(
                                    "Skipping state at index {next_state_index} because of filter"
                                );

                                // Move to the next state.
                                let next_next_index = match self.scheduler.next_state(
                                    next_state_index,
                                    &self.states,
                                    &context,
                                ) {
                                    Ok(idx) => idx,
                                    Err(SchedulerError::NoNextState) => {
                                        return Ok(context);
                                    }
                                    Err(err) => return Err(StateMachineError::SchedulerError(err)),
                                };

                                state_index = next_next_index;
                                last_state_result = Some(Ok(()));
                                continue;
                            }
                        }

                        // Execute the state handler.
                        let result = state.handler(context.clone()).await;

                        // Track the execution.
                        let _ = self.tracker.as_mut().track(
                            Index::new(next_state_index, state.label(), state.tags()),
                            context.clone(),
                        );

                        // Continue to the next state.
                        state_index = next_state_index;
                        last_state_result = Some(result);
                        continue;
                    }

                    // Parallel execution path.
                    let states = self.states.clone();
                    let task_ctx = context.clone();
                    let mut results = futures::future::join_all(
                        next_state_indexes
                            .into_iter()
                            .filter(|&i| self.has_state(i))
                            .map(|next_state_index| {
                                let states = states.clone();
                                let ctx = task_ctx.clone();
                                async move {
                                    let state = &states[next_state_index];
                                    let label = state.label();
                                    let tags = state.tags();
                                    let result = state.handler(ctx.clone()).await;
                                    let snapshot = ctx.snapshot().unwrap_or(ctx);
                                    (next_state_index, label, tags, snapshot, result)
                                }
                            }),
                    )
                    .await;

                    results.sort_by_key(|(idx, _, _, _, _)| *idx);

                    let mut first_error: Option<(usize, StateResult)> = None;
                    let mut last_executed_index = state_index;
                    for (idx, label, tags, snapshot, result) in results {
                        last_executed_index = idx;

                        // Track the execution (using a snapshot captured at completion time).
                        let _ = self
                            .tracker
                            .as_mut()
                            .track(Index::new(idx, label, tags), snapshot);

                        if first_error.is_none() {
                            if let Err(e) = result {
                                first_error = Some((idx, Err(e)));
                            }
                        }
                    }

                    if let Some((failed_index, failed_result)) = first_error {
                        state_index = failed_index;
                        last_state_result = Some(failed_result);
                        continue;
                    }

                    state_index = last_executed_index;
                    last_state_result = Some(Ok(()));
                    continue;
                }
                Err(StateMachineError::SchedulerError(SchedulerError::NoNextState)) => {
                    return Ok(context);
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Execute the state machine with the given SafeContext
    pub async fn execute(
        &mut self,
        context: SafeContext,
    ) -> Result<SafeContext, StateMachineError> {
        self.execute_rec(context, 0, None).await
    }

    /// Execute the state machine starting from a specific state index
    pub async fn execute_from(
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

        self.execute_rec(context, start_index, None).await
    }

    /// Execute the state machine, filtering out states with specific tags
    pub async fn execute_with_filter(
        &mut self,
        context: SafeContext,
        filter_tags: Vec<crate::state::Tag>,
    ) -> Result<SafeContext, StateMachineError> {
        self.scheduler.set_filter_tags(Some(filter_tags));
        let result = self.execute(context).await;
        self.scheduler.set_filter_tags(None);
        result
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::state::safe_context::{create_default_safe_context, SafeContext};
    use crate::state::{
        standard_tags, StateError, StateErrorRecoverability, StateHandler, StateResult,
    };
    use serde_derive::{Deserialize, Serialize};

    #[mfm_machine_derive_legacy::state_handler(
        label = "setup_state",
        tags = ["setup", "config"],
        depends_on = [],
        strategy = Latest
    )]
    #[derive(Debug, Clone, PartialEq)]
    pub struct Setup;

    #[derive(Serialize, Deserialize)]
    struct SetupCtx {
        a: String,
        b: u32,
    }

    #[async_trait::async_trait]
    impl StateHandler for Setup {
        async fn handler(&self, context: SafeContext) -> StateResult {
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

    #[mfm_machine_derive_legacy::state_handler(
        label = "report_state",
        tags = ["report"],
        depends_on = ["setup"],
        strategy = Latest
    )]
    #[derive(Debug, Clone, PartialEq)]
    pub struct Report;

    #[async_trait::async_trait]
    impl StateHandler for Report {
        async fn handler(&self, context: SafeContext) -> StateResult {
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

    #[tokio::test]
    async fn test_state_machine_simple_flow() {
        let setup = Setup::new();
        let report = Report::new();

        let states: Vec<Box<dyn StateHandler>> = vec![
            Box::new(setup) as Box<dyn StateHandler>,
            Box::new(report) as Box<dyn StateHandler>,
        ];
        let states = Arc::from(states);

        println!("Creating state machine...");
        let mut state_machine = super::StateMachine::new(states).unwrap();

        println!("Creating context...");
        let safe_context = create_default_safe_context();

        println!("Executing state machine...");
        let result = state_machine.execute(safe_context).await;

        println!("Result: {result:?}");
        assert!(result.is_ok());

        let safe_result = result.unwrap();
        println!("Reading report data...");
        let report_data: ReportCtx = safe_result.read_typed("report").unwrap();
        println!("Report data: {report_data:?}");
        assert_eq!(report_data.a, "setup_b_handled");
        assert_eq!(report_data.c, 2);
    }

    #[tokio::test]
    async fn test_state_machine_with_filter() {
        let setup = Setup::new();
        let report = Report::new();

        let states: Vec<Box<dyn StateHandler>> = vec![
            Box::new(setup) as Box<dyn StateHandler>,
            Box::new(report) as Box<dyn StateHandler>,
        ];
        let states = Arc::from(states);

        println!("Creating state machine...");
        let mut state_machine = super::StateMachine::new(states).unwrap();

        println!("Creating context...");
        let safe_context = create_default_safe_context();

        println!("Setting filter tags: {:?}", standard_tags::report());
        // Use execute_with_filter
        let result = state_machine
            .execute_with_filter(safe_context, vec![standard_tags::report()])
            .await;

        println!("Result: {result:?}");
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
