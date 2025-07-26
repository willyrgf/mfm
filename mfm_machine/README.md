# MFM State Machine Documentation

## Overview

The `mfm_machine` is a generic, asynchronous, and recoverable state machine designed for building complex, multi-step workflows. It provides a robust framework for executing a series of operations (states) where each state can depend on the output of others, and the entire process can gracefully recover from transient errors.

It is particularly well-suited for applications that involve orchestrating tasks like data fetching, computation, and interaction with external systems (e.g., blockchains, APIs), where reliability and fault tolerance are critical.

## Design Philosophy

- **Tag-Based & Declarative**: States are self-contained units that declare their identity (`Label`) and capabilities (`Tag`) through metadata. The `Scheduler` uses these tags to determine the execution order, allowing for both simple sequential workflows and complex dependency graphs.
- **Recoverable by Default**: The state machine is designed for resilience. It tracks the execution history and context at each step. If a recoverable error occurs, the `ErrorHandler` can rewind the machine to a previously successful state and retry, preventing the entire workflow from failing due to transient issues.
- **Extensible & Composable**: Key components like the `Scheduler`, `Tracker`, and `ErrorHandler` are implemented as traits. This allows developers to provide custom implementations to tailor the machine's behavior for specific needs, such as custom scheduling logic or error handling strategies.
- **Immutable & Thread-Safe Context**: State data is managed in a `SafeContext`, a thread-safe wrapper that ensures data integrity. States do not modify the context directly; instead, they produce a new context with the updated data, promoting functional purity and making workflows easier to reason about.
- **Type-Safe Data Handling**: The `TypedContext` trait allows states to read and write strongly-typed data, eliminating the need for manual JSON parsing and reducing the risk of runtime errors.

## Architecture

The `mfm_machine` crate is organized into two primary modules, separating the definition of a "state" from the machinery that executes it.

```
mfm_machine/
├── src/
│   ├── lib.rs              # Crate entry point
│   ├── state/
│   │   ├── mod.rs          # Core traits: StateHandler, StateMetadata, Tag, Label, StateError
│   │   ├── context.rs      # Defines the Context and TypedContext traits for data management
│   │   └── safe_context.rs # A thread-safe, lock-based wrapper for the context
│   └── state_machine/
│       ├── mod.rs          # The StateMachine engine and builder
│       ├── scheduler.rs    # Defines Scheduler and ErrorHandler traits for flow control
│       └── tracker.rs      # Defines the Tracker trait for recording execution history
└── tests/
    └── ...                 # Unit and integration tests demonstrating usage
```

## Core Components

### `StateHandler` Trait

The `StateHandler` is the fundamental building block of the state machine. It represents a single, executable step in a workflow. Every state must implement this trait.

```rust
pub trait StateHandler: StateMetadata + Send + Sync {
    // Executes the state's logic, receiving and potentially updating the context.
    fn handler(&self, context: SafeContext) -> StateResult;
}
```

### `StateMetadata` Trait

This trait provides descriptive information about a state, which the `Scheduler` and `ErrorHandler` use to manage the workflow. The `mfm_machine_derive::StateMetadataReqs` macro can auto-generate the implementation.

- **`label()`**: A unique `Label` for the state.
- **`tags()`**: A `Vec<Tag>` that categorizes the state (e.g., `config`, `fetch_data`, `compute`).
- **`depends_on()`**: A `Vec<Tag>` declaring which other states must run before this one.
- **`depends_on_strategy()`**: Defines how dependencies are resolved (e.g., `Latest`, `Earliest`).

### `SafeContext`

This is a thread-safe container for the data that flows through the state machine. It acts as a shared memory bus that states can read from and write to.

- **`read_typed<T>(&self, key: &str) -> Result<T>`**: Deserializes a value from the context into a specific type.
- **`write_typed<T>(&self, key: &str, value: &T) -> Result<()>`**: Serializes a value and writes it to the context.

### `StateMachine`

The engine that orchestrates the execution of states. It is configured using the `StateMachineBuilder`.

```rust
// Create a set of states
let states: States = Arc::new([
    Box::new(StateA::new()),
    Box::new(StateB::new()),
]);

// Create a new state machine with default components
let mut state_machine = StateMachine::new(states);

// Create an initial context
let context = create_default_safe_context();

// Execute the workflow
let final_context = state_machine.execute(context)?;
```

## Usage Example

Here is a simple example of a two-state workflow: `Setup` and `Report`.

**1. Define the States**

First, define the structs for your states and use the derive macro to handle the metadata.

```rust
use mfm_machine::state::{
    safe_context::SafeContext, DependencyStrategy, Label, StateHandler, StateMetadata, StateResult, Tag
};
use mfm_machine_derive::StateMetadataReqs;
use serde_derive::{Serialize, Deserialize};

// --- State 1: Setup ---

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
            label: Label::new_const("setup_state"),
            tags: vec![Tag::new_const("setup")],
            depends_on: vec![], // No dependencies
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SetupCtx {
    value: u32,
}

impl StateHandler for Setup {
    fn handler(&self, context: SafeContext) -> StateResult {
        let data = SetupCtx { value: 42 };
        // Write the data to the context for other states to use
        context.write_typed("setup_data", &data).map_err(|e| e.into())
    }
}

// --- State 2: Report ---

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
            label: Label::new_const("report_state"),
            tags: vec![Tag::new_const("report")],
            depends_on: vec![Tag::new_const("setup")], // Depends on the Setup state
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for Report {
    fn handler(&self, context: SafeContext) -> StateResult {
        // Read data produced by the Setup state
        let setup_data: SetupCtx = context.read_typed("setup_data")?;
        println!("Report state received value: {}", setup_data.value);
        Ok(())
    }
}
```

**2. Run the State Machine**

Create instances of your states, build the state machine, and execute it.

```rust
use mfm_machine::state::safe_context::create_default_safe_context;
use mfm_machine::state::States;
use mfm_machine::state_machine::StateMachine;
use std::sync::Arc;

// Create a list of states to execute
let states: States = Arc::new([
    Box::new(Setup::new()),
    Box::new(Report::new()),
]);

// Instantiate the state machine
let mut state_machine = StateMachine::new(states);

// Create the initial context
let context = create_default_safe_context();

// Run the workflow
let result = state_machine.execute(context);

assert!(result.is_ok());
```

## Error Handling and Recovery

The state machine's error handling is designed to be robust and transparent.

- **`StateError` Enum**: When a state's `handler` fails, it should return a `StateError`. This enum classifies the error (e.g., `RpcConnection`, `StorageAccess`, `ParsingInput`) and specifies its `StateErrorRecoverability`.
- **`StateErrorRecoverability`**: An error can be marked as `Recoverable` or `Unrecoverable`.
  - `Unrecoverable` errors will immediately halt the state machine.
  - `Recoverable` errors trigger the recovery mechanism.
- **Recovery Process**: When a recoverable error occurs, the `StateMachine` consults its `ErrorHandler`. The `DefaultErrorHandler` inspects the failed state's dependencies (`depends_on` tags) and uses the `Tracker` to find the last successfully executed state that satisfies the dependency. It then restores the context from that point and re-runs the workflow from the recovered state. This allows the machine to automatically retry operations that might have failed due to transient network issues or other temporary problems.
