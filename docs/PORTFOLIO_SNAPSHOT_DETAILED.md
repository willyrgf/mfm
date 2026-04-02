# Portfolio Snapshot Detailed Execution Map

This document explains the current end-to-end execution of the portfolio snapshot flow in the
current repository state.

Primary command surfaces covered here:

- `nix run .#mfm::portfolio::snapshot -- /tmp/portfolio-request.json`
- `mfm_cli --output-format json portfolio snapshot --request-file /tmp/portfolio-request.json`
- `mfm_cli --output-format json portfolio snapshot --request-file /tmp/portfolio-config.toml`

The current code is authoritative. If a doc disagrees with code, code wins until the doc is
updated.

Useful companion documents:

- [`docs/architecture.md`](architecture.md)
- [`docs/design.md`](design.md)
- [`docs/ops-and-states.md`](ops-and-states.md)
- [`docs/helios.md`](helios.md)
- [`docs/evm-rpc-routing.md`](evm-rpc-routing.md)
- [`bin/cli/README.md`](../bin/cli/README.md)

## 1. Entry points

### What `.#mfm::portfolio::snapshot` is in Nixfied terms

`mfm::portfolio::snapshot` is a Nixfied app id exposed from `nixfied/project/module.nix`:

- [`nixfied/project/module.nix`](../nixfied/project/module.nix) defines the app in `apps."mfm::portfolio::snapshot"`.
- That app is created by `mkTaskApp`, so it is `kind = "taskRef"`.
- The referenced task id is `task.mfm.portfolio.snapshot`.

The task itself is also defined in [`nixfied/project/module.nix`](../nixfied/project/module.nix):

- It is created with `mkCommandTask`.
- The runner type is `shell`.
- The task contract declares JSON output on stdout.
- The task shell body is the public operational wrapper for this flow.

So, in Nixfied terms:

1. app id: `mfm::portfolio::snapshot`
2. app kind: `taskRef`
3. target task id: `task.mfm.portfolio.snapshot`
4. task runner kind: `shell`

### How it maps to the task app and wrapper

At launch time, Nixfied materializes the `taskRef` app into an executable runtime app that runs:

```sh
nixfied-orchestrator run-task task.mfm.portfolio.snapshot
```

That happens through the framework launch/runtime stack:

- `nix run` builds a selected launcher from `framework/core/mkFlakeOutputs.nix`
- the launcher resolves `compiled.apps.${appName}` in `framework/launch/run-selected-app.nix`
- `framework/core/materializeExecution.nix` turns the `taskRef` into a runtime script
- that runtime script `exec`s `nixfied-orchestrator run-task ...`

The shell wrapper inside `task.mfm.portfolio.snapshot` then:

- validates the request-file invocation contract
- manages Postgres lifecycle
- optionally manages Helios lifecycle
- exports runtime env such as `DATABASE_URL` and managed RPC bootstrap env
- resolves the packaged `mfm_cli` binary
- runs the inner Rust CLI
- validates the returned JSON envelope with `jq`
- replays the validated stdout payload unchanged

### How this relates to `mfm_cli portfolio snapshot`

The relationship is important:

- `mfm::portfolio::snapshot` is not the Rust implementation itself.
- It is an outer operational wrapper around the Rust implementation.
- The actual domain execution happens inside the packaged `mfm_cli` binary.

Also note:

- the snapshot task does not call a flake app passthrough wrapper
- it does not use `cargo run`
- it resolves the packaged binary path `${conf.packages."mfm-cli"}/bin/mfm_cli`

The package is defined in [`nixfied/project/conf.nix`](../nixfied/project/conf.nix) as a Rust
package that builds `-p mfm --bin mfm_cli`.

So the outer task and inner CLI are separate layers:

- outer layer: Nixfied shell wrapper and service orchestration
- inner layer: Rust CLI -> app services -> op planner -> machine runtime -> states

## 2. Runtime layers

This is the full layer stack, from `nix run` down to the actual portfolio report JSON.

### Nix launcher layer

Owned by vendored Nixfied framework code.

Responsibilities:

- find flake root
- build a selected launcher for the requested app
- pass command-line args through unchanged

Key files:

- [`nixfied/framework/core/mkFlakeOutputs.nix`](../nixfied/framework/core/mkFlakeOutputs.nix)
- [`nixfied/framework/launch/run-selected-app.nix`](../nixfied/framework/launch/run-selected-app.nix)

### Selected app layer

This is where the app id `mfm::portfolio::snapshot` resolves to the compiled app definition in the
Nixfied model.

Responsibilities:

- resolve `compiled.apps."mfm::portfolio::snapshot"`
- choose the right runtime program for the selected app

Key files:

- [`nixfied/project/module.nix`](../nixfied/project/module.nix)
- [`nixfied/framework/launch/run-selected-app.nix`](../nixfied/framework/launch/run-selected-app.nix)

### App runtime layer

This is the generated program for manifest-backed apps/tasks.

Responsibilities:

- convert the `taskRef` app into a runtime shell app
- call `nixfied-orchestrator run-task task.mfm.portfolio.snapshot`

Key file:

- [`nixfied/framework/core/materializeExecution.nix`](../nixfied/framework/core/materializeExecution.nix)

### Orchestrator layer

The Nixfied orchestrator prepares the runtime environment around task execution.

Responsibilities:

- set `NIXFIED_EXECUTOR_BIN`
- resolve registry/artifacts roots
- export runtime paths and bookkeeping dirs
- dispatch `run-task`

Key file:

- [`nixfied/framework/runtime/orchestrator.nix`](../nixfied/framework/runtime/orchestrator.nix)

### Executor layer

The executor actually runs the modeled task.

Responsibilities:

- resolve runner type (`shell` here)
- resolve task runtime plan
- run pre-hooks and post-hooks
- execute the shell command in the modeled runtime

Key file:

- [`nixfied/framework/runtime/executor.nix`](../nixfied/framework/runtime/executor.nix)

### Task shell/wrapper layer

This is the outer portfolio snapshot wrapper in project-owned Nix code.

Responsibilities:

- request-file validation
- service policy resolution
- Postgres readiness/start/setup
- Helios optional readiness/start
- `DATABASE_URL` construction
- managed RPC bootstrap env setup
- packaged `mfm_cli` execution
- final JSON envelope validation

Key file:

- [`nixfied/project/module.nix`](../nixfied/project/module.nix)

### Rust CLI layer

This is the inner Rust command-line entrypoint.

Responsibilities:

- initialize observability
- parse args with Clap
- dispatch to `portfolio snapshot`
- render stable JSON/text output contracts

Key files:

- [`bin/cli/src/main.rs`](../bin/cli/src/main.rs)
- [`bin/cli/src/lib.rs`](../bin/cli/src/lib.rs)
- [`bin/cli/src/commands/mod.rs`](../bin/cli/src/commands/mod.rs)
- [`bin/cli/src/commands/portfolio/mod.rs`](../bin/cli/src/commands/portfolio/mod.rs)
- [`bin/cli/src/commands/portfolio/snapshot.rs`](../bin/cli/src/commands/portfolio/snapshot.rs)
- [`bin/cli/src/presentation/output.rs`](../bin/cli/src/presentation/output.rs)

### App services layer

This is the shared application facade used by CLI and REST.

Responsibilities:

- build stores from env/args
- build the engine bundle
- expose `start_portfolio_snapshot()`

Key files:

- [`bin/cli/src/support/run_stores.rs`](../bin/cli/src/support/run_stores.rs)
- [`bin/cli/src/support/app_services.rs`](../bin/cli/src/support/app_services.rs)
- [`crates/app/src/lib.rs`](../crates/app/src/lib.rs)

### Op planning layer

This layer turns a validated request into a deterministic state graph.

Responsibilities:

- validate request bundle
- encode request as `op_config`
- lower `portfolio_execute/v1` into a fixed semantic graph

Key files:

- [`crates/app/src/lib.rs`](../crates/app/src/lib.rs)
- [`crates/ops/portfolio-tracker-op/src/lib.rs`](../crates/ops/portfolio-tracker-op/src/lib.rs)
- [`crates/ops/portfolio-tracker-op/src/plan.rs`](../crates/ops/portfolio-tracker-op/src/plan.rs)
- [`crates/ops/portfolio-tracker-op/src/plan_ops.rs`](../crates/ops/portfolio-tracker-op/src/plan_ops.rs)

### State-machine / execution layer

This is the machine runtime.

Responsibilities:

- persist manifest
- persist optional compiled execution spec
- create run id
- append kernel events
- write context snapshots
- execute states sequentially
- finalize run

Key files:

- [`crates/sdk/src/unstable.rs`](../crates/sdk/src/unstable.rs)
- [`crates/machine/src/runtime.rs`](../crates/machine/src/runtime.rs)
- [`crates/machine/src/runtime/attempt.rs`](../crates/machine/src/runtime/attempt.rs)
- [`crates/machine/src/context_runtime.rs`](../crates/machine/src/context_runtime.rs)

### Storage / artifact layer

This flow uses several storage surfaces:

- PostgreSQL-backed run stream store for `run:*`
- filesystem artifact store for manifest/context/output artifacts
- Postgres-backed `rpc.control` control-plane store by default

Artifact kinds involved:

- manifest artifact
- compiled execution spec artifact
- context snapshot artifacts
- output artifact for the canonical portfolio snapshot

Key files:

- [`bin/cli/src/support/run_stores.rs`](../bin/cli/src/support/run_stores.rs)
- [`crates/machine/src/context_runtime.rs`](../crates/machine/src/context_runtime.rs)
- [`crates/states/common/src/output.rs`](../crates/states/common/src/output.rs)
- [`crates/transports/rpc-control/src/lib.rs`](../crates/transports/rpc-control/src/lib.rs)
- [`crates/storages/control-plane-postgres/src/lib.rs`](../crates/storages/control-plane-postgres/src/lib.rs)

### External dependency layer

The portfolio flow ultimately depends on:

- local Postgres service
- optional local Helios service
- Helios upstream execution/consensus endpoints when Helios is used in mainnet mode
- caller-supplied managed RPC sources when Helios is skipped or unavailable

## 3. Exact execution sequence

This is the full ordered path for:

```sh
nix run .#mfm::portfolio::snapshot -- /tmp/portfolio-request.json
```

### Build-time

Before any specific invocation, the repo defines a packaged CLI binary in
`conf.packages."mfm-cli"`. That package builds Rust crate `mfm` and binary `mfm_cli`.

This is distinct from launch-time and task runtime.

### Launch-time

1. `nix run .#mfm::portfolio::snapshot -- /tmp/portfolio-request.json` enters the flake app
   launcher path.
2. The framework launcher in `mkFlakeOutputs.nix` builds a selected launcher from
   `framework/launch/run-selected-app.nix`.
3. `run-selected-app.nix` recompiles the Nixfied graph, resolves
   `compiled.apps."mfm::portfolio::snapshot"`, and `exec`s that selected app program.
4. `materializeExecution.nix` has already materialized the `taskRef` app as a runtime script that
   runs:

   ```sh
   nixfied-orchestrator run-task task.mfm.portfolio.snapshot
   ```

5. The orchestrator exports runtime env and points `NIXFIED_EXECUTOR_BIN` at
   `nixfied-executor`.
6. The executor resolves the task runner as `shell` and runs the task command body from
   `task.mfm.portfolio.snapshot`.

### Task runtime before Rust starts

7. The snapshot shell wrapper enables `set -euo pipefail`.
8. It sources packaged CLI resolver code and service-policy helpers.
9. It supports `--help` and otherwise requires exactly one positional argument.
10. It stores the argument in `request_file`.
11. If `request_file` is relative, it is rewritten as `$PWD/$request_file`.
12. It sets `snapshot_database="${postgresDatabase}"`, which resolves to `mfm` from
    `conf.services.postgres.database`.
13. It checks that the request file exists and is readable.
14. It requires `POSTGRES_PORT` to already be available in the task runtime.

### Service policy resolution

15. The wrapper rejects the removed envs `MFM_KEEP_SERVICES` and `SERVICE_REUSE_POLICY`.
16. It requires `SVC_POSTGRES_FULL_START`, `SVC_POSTGRES_READY`, `SVC_POSTGRES_STOP`, and
    `SVC_POSTGRES_SETUP_DB`.
17. It initializes:

    - `owner_scope="${SERVICE_OWNER_SCOPE:-}"`
    - `discovery_scope="${SERVICE_DISCOVERY_SCOPE:-}"`

18. If `owner_scope` is empty, it derives it from `discovery_scope`:

    - `global -> persistent`
    - `local -> ephemeral`
    - otherwise defaults to `ephemeral`

19. If `discovery_scope` is empty, it derives it from `owner_scope`:

    - `persistent -> global`
    - otherwise defaults to `local`

20. It validates the final owner/discovery matrix with `nixfied_policy_validate_matrix`.
21. It derives `policy_mode` with `nixfied_policy_infer_reuse_policy`.
22. If `owner_scope = persistent`, cleanup is disabled and the wrapper leaves owned services
    running after exit.
23. Otherwise cleanup is enabled and owned services are stopped on exit.

### Temporary files and traps

24. The wrapper creates:

    ```sh
    result_file="$(mktemp "${TMPDIR:-/tmp}/mfm-portfolio-snapshot.XXXXXX.json")"
    ```

25. It installs traps:

    - `EXIT` cleans up `result_file` and stops owned services if cleanup is enabled
    - `INT` exits `130`
    - `TERM` exits `143`

### Postgres readiness / startup / setup behavior

26. The wrapper first tries `"$SVC_POSTGRES_READY"`.
27. If readiness succeeds, it reuses the running Postgres instance on `POSTGRES_PORT`.
28. If readiness fails, it marks Postgres as owned, then runs:

    ```sh
    "$SVC_POSTGRES_FULL_START"
    "$SVC_POSTGRES_READY"
    ```

29. `SVC_POSTGRES_FULL_START` comes from the framework Postgres service contract:

    - `full-start` uses `start` as a pre-op
    - `start` runs init/check-config/preflight/start
    - `full-start-leaf` then runs `setupDb`

30. The service runtime resolves `PGPORT`, `PGDATA`, `PGSOCKET_DIR`, and `PGDATABASE` from slot
    info before start/readiness/setup.
31. `startLeaf` reuses a matching running instance when possible, removes stale pid files, or runs
    `pg_ctl` and waits until `pg_isready` succeeds.
32. `ready` executes the configured readiness probe plan.
33. `setupDb` creates the `postgres` role if missing, creates the requested database if missing, and
    creates configured extensions if any.
34. After reuse or start, the snapshot wrapper always runs:

    ```sh
    PGDATABASE="$snapshot_database" "$SVC_POSTGRES_SETUP_DB"
    ```

35. That extra setup step is intentional. A reused Postgres slot can already be healthy but still
    not contain the `mfm` database the snapshot task expects.

### Helios skipped vs available behavior

36. The wrapper initializes `helios_available=0`.
37. It sets `helios_available=1` only if:

    - `is_service_skipped helios` is false
    - `SVC_HELIOS_FULL_START` exists
    - `SVC_HELIOS_READY` exists

38. `is_service_skipped helios` is driven by `SKIP_HELIOS` through Nixfied skip-policy helpers.

### Helios available path

39. If Helios is available, the wrapper requires `HELIOSRPC_PORT`.
40. It exports:

    - `HELIOS_NETWORK="${HELIOS_NETWORK:-mainnet}"`
    - `HELIOS_EXECUTION_RPC_URL="${HELIOS_EXECUTION_RPC_URL:-<project default>}"`
    - `HELIOS_CONSENSUS_RPC_URL="${HELIOS_CONSENSUS_RPC_URL:-<project default>}"`
    - `HELIOS_CHECKPOINT="${HELIOS_CHECKPOINT:-<project default>}"`

41. The project defaults come from `conf.services.helios`:

    - `network = "local"` at the service definition level
    - `executionRpcUrl = "https://ethereum-rpc.publicnode.com"`
    - `consensusRpcUrl = "https://lodestar-mainnet.chainsafe.io"`
    - `defaultConsensusRpcUrl = "https://lodestar-mainnet.chainsafe.io"`
    - `checkpoint = ""`

42. The task wrapper overrides the service-level default network by forcing mainnet for this public
    command.
43. If `HELIOS_NETWORK != mainnet`, the task aborts immediately.
44. The wrapper then tries `"$SVC_HELIOS_READY"`.
45. If readiness succeeds, it reuses the running Helios instance.
46. If readiness fails, it marks Helios as owned and runs:

    ```sh
    "$SVC_HELIOS_FULL_START"
    HELIOS_READY_TIMEOUT_SECS=...
    HELIOS_READY_INTERVAL_SECS=...
    "$SVC_HELIOS_READY"
    ```

47. Inside the Helios service module:

    - runtime prelude resolves `HELIOS_RPC_PORT`, `HELIOS_EXECUTION_PORT`, service dirs, and env
    - `checkConfig` validates binary and required env
    - `startPrepareBody` derives a weak-subjectivity checkpoint if needed for non-local networks
    - `startCommand` launches `helios ethereum --network ... --rpc-port ...`
    - `ready` waits for pid file creation and then runs repeated JSON-RPC readiness probes

48. The snapshot wrapper then seeds managed RPC bootstrap env if the caller did not already set it:

    - `MFM_EVM_RPC_SOURCES_JSON=[{"id":"helios_local","network_id":"ethereum-mainnet","rpc_url":"http://127.0.0.1:$HELIOSRPC_PORT","kind":"local"}]`
    - `MFM_EVM_RPC_PREFERRED_ORDER=helios_local`

49. If the caller already set `MFM_EVM_RPC_SOURCES_JSON`, the task leaves it alone even if Helios
    is running.

### Helios skipped or unavailable path

50. If Helios is skipped or unavailable, the wrapper requires `MFM_EVM_RPC_SOURCES_JSON`.
51. If that env var is missing, the task aborts before Rust starts.
52. If it is present, the task does not modify it.
53. In that path, the flow still works as long as the managed RPC source catalog contains valid
    sources for the requested networks.

### `DATABASE_URL` construction and handoff

54. After service setup, the wrapper exports:

    ```sh
    DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/$snapshot_database"
    ```

55. With current project config, `snapshot_database` is `mfm`, so the normal value is:

    ```sh
    postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm
    ```

56. It resolves the packaged binary path with `resolve_packaged_mfm_cli_binary()`.
57. It runs:

    ```sh
    "$mfm_cli_bin" --output-format json portfolio snapshot --request-file "$request_file" >"$result_file"
    ```

### Rust CLI startup and request parsing

58. `bin/cli/src/main.rs` starts Tokio and calls `mfm::run().await`.
59. `bin/cli/src/lib.rs` initializes observability and parses the CLI with Clap.
60. `bin/cli/src/commands/mod.rs` dispatches to `Commands::Portfolio`.
61. `bin/cli/src/commands/portfolio/mod.rs` dispatches `PortfolioCommand::Snapshot`.
62. `snapshot::execute()` calls `execute_internal()`.
63. `parse_request()` handles the transport boundary directly.
64. The CLI transport parser:

    - rejects both `--request-json` and `--request-file` together
    - rejects neither provided
    - reads the file with `std::fs::read_to_string`
    - parses authored JSON or TOML via `mfm-portfolio-config`
    - canonicalizes authored config into the typed canonical request before app handoff
    - deserializes `PortfolioSnapshotRequest` from JSON

### Store creation and app service wiring

65. `make_app_services_from_args()` calls `make_stores()`.
66. `make_stream_store()` requires `DATABASE_URL` from CLI args or env and connects
    `PostgresStreamStore::connect(&database_url)`.
67. `make_artifact_store()` uses `--artifact-root`, `MFM_ARTIFACT_ROOT`, or defaults to:

    ```text
    $HOME/.mfm/run_artifacts
    ```

68. `make_app_services()` builds `AppServices::new(make_engine_bundle(), streams, artifacts)`.

### Engine bundle construction

69. `AppBuilder::build()` registers operations:

    - all public built-in root ops
    - `portfolio_config_build` public build op
    - `portfolio_execute` public op
    - `portfolio_tracker` public compatibility op
    - portfolio planner-internal child ops

70. It registers transports:

    - proof
    - exec
    - nix flake
    - local fs
    - local evm
    - local keystore
    - `RpcControlTransportFactory::from_env()`

71. `RpcControlTransportFactory::from_env()` parses bootstrap source env:

    - `MFM_EVM_RPC_SOURCES_JSON`
    - `MFM_EVM_RPC_PREFERRED_ORDER`
    - `MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS`

72. The engine bundle then creates:

    - operation registry
    - pipeline planner
    - plan resolver
    - router live transport factory
    - child-run live transport wrapper
    - `DefaultExecutionEngine`

### Request validation and conversion into a run

73. `AppServices::start_portfolio_snapshot()` serializes the canonical request into `op_config`.
75. It starts a single-op run:

    - `op_id = "portfolio_tracker"`
    - `op_version = "v1"`

76. `start_run()` wraps that as a one-step pipeline with `machine_id = portfolio_tracker` and
    `step_id = main`.
77. `default_run_config()` sets current runtime behavior:

    - `io_mode = Live`
    - `max_attempts = 1`
    - `execution_mode = Sequential`
    - `context_checkpointing = AfterEveryState`

`AppServices::start_portfolio_config_build()` now exposes the explicit build-only public boundary.
It starts `portfolio_config_build/v1`, which consumes the same canonical portfolio request shape
and publishes:

- the typed built config
- the canonical config artifact id
- the built config artifact id
- a stable build report

`start_portfolio_snapshot()` no longer calls the pure build helper directly. The canonical request
now enters the planner through `portfolio_tracker/v1`, which keeps the CLI and feature contract the
same while moving the canonical-to-built handoff into the op layer.

### How the request becomes an op/state graph

78. `DefaultRunLauncher::start_pipeline()` asks the planner to build planned execution.
79. The planner validates the pipeline, resolves the root op from the registry, and calls
    `PortfolioTrackerOp::expand()`.
80. For canonical input, `PortfolioTrackerOp::expand()` inserts `portfolio_config_build` as the
    first child workflow and wires `portfolio_execute` from the build child's `/built_config`
    planner payload.
    The compatibility seam described in the RFC is now closed: the build child is authoritative for
    artifact/report publication and for execute-child input materialization.
81. For built input, `PortfolioTrackerOp::expand()` skips the build child and lowers directly into
    the semantic execution child ops.
82. The execution portion of that graph lowers the semantic spec into child ops:

    - `prepare_execution_sources`
    - `resolve_subjects`
    - `pin_execution_views`
    - `resolve_valuation_inputs`
    - one `observe_*` child per compiled observation batch
    - `merge_observations`
    - `assemble_snapshot`
    - `project_report`

84. It wires explicit imports/exports between child ops and re-exports:

    - `snapshot`
    - `snapshot_artifact_id`
    - `report`

85. Each internal child op in `plan_ops.rs` lowers to one leaf state in
    `crates/states/portfolio/src/execution_states.rs`.

### Manifest and compiled execution spec persistence

86. `DefaultRunLauncher` serializes `PipelineManifestInput { pipeline, input }`.
87. It builds `RunManifest { op_id, op_version, input_params, run_config, build }`.
88. It canonical-JSON encodes the manifest and persists it as `ArtifactKind::Manifest`.
89. It also persists an optional compiled execution spec artifact as
    `ArtifactKind::Other("compiled_execution_spec")`.
90. When present, the compiled spec artifact id is written into initial context key:

    ```text
    mfm.compiled_execution_spec_artifact_id
    ```

### Machine start and initial checkpoint

91. `DefaultExecutionEngine::start()` verifies that the manifest artifact exists.
92. It creates `run_id = Uuid::new_v4()`.
93. It dumps the initial context and writes it as a content-addressed
    `ArtifactKind::ContextSnapshot`.
94. It creates an event writer on stream family `run:<run_id>`.
95. It appends `KernelEvent::RunStarted { op_id, manifest_id, initial_snapshot_id }`.

### State execution sequence

96. `run_states()` topologically sorts the flattened execution plan.
97. Because current execution mode is sequential, states run one by one in topological order.
98. For each state attempt:

    - append `StateEntered`
    - load the base context snapshot
    - create a staged overlay context
    - run the state handler with live IO
    - on success, dump a new context snapshot and append `StateCompleted`
    - on failure, append `StateFailed`

99. In this flow, the state sequence is:

    - `PrepareExecutionSourcesState`
    - `ResolveSubjectsState`
    - `PinExecutionViewsState`
    - `ResolveValuationInputsState`
    - one or more `ObserveCompiledBatchState`
    - `MergeObservationsState`
    - `AssembleSnapshotState`
    - `ProjectReportState`

### `PrepareExecutionSourcesState` and managed RPC behavior

100. `PrepareExecutionSourcesState` sorts source-preparation tasks deterministically.
101. For EVM tasks it decodes payload containing:

    - `network_id`
    - optional `control_scope`

102. Empty `control_scope` is normalized to `shared`.
103. It creates `EvmIoClient::new(self.state_id.clone(), io)`.
104. It calls:

    ```rust
    client.prepare_sources_in_scope(control_scope, network_id)
    ```

105. The typed collector crate derives a deterministic fact key for the `rpc.control` request using
     canonical JSON of:

    ```json
    {
      "kind": "prepare_sources",
      "control_scope": "...",
      "network_id": "..."
    }
    ```

106. The `rpc.control` transport receives that call and uses the bootstrap catalog from
     environment.
107. `MFM_EVM_RPC_SOURCES_JSON` is parsed into `RpcControlBootstrapSource` objects.
108. `MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS` upgrades matching source entries so they must pass
     `eth_getProof` probing.
109. `MFM_EVM_RPC_PREFERRED_ORDER` seeds ranking order. If unset, the source list order is used.
110. The transport filters candidate sources by `network_id`.
111. It uses a `ControlPlaneStore` in `PostgresEnv` mode by default, which lazily connects a
     `ControlPlanePostgresStore` using the same `DATABASE_URL`.
112. The control-plane store ensures tables:

    - `mfm_streams`
    - `mfm_stream_records`
    - `mfm_rpc_source_state`
    - `mfm_source_pool_state`

113. `prepare_sources_impl()` then:

    - ensures the declared catalog snapshot for `(control_scope, network_id, pool_kind)`
    - persists membership when needed
    - probes stale or missing sources
    - computes health and ranking
    - persists ranked order when needed
    - returns `PrepareSourcesResponse`

114. Ranking prefers:

    - healthy over unhealthy
    - required capability satisfied over unsatisfied
    - not cooling down
    - fewer failures
    - lower latency
    - source kind rank (`local`, then `remote_user`, then `remote_public`)
    - preferred order
    - source id

115. `PrepareExecutionSourcesState` rejects the response if no source is healthy.

### Remaining semantic runtime states

116. `ResolveSubjectsState` reads prepared sources from context and resolves semantic subjects using
     planner-selected subject runtime adapters.
117. `PinExecutionViewsState` reads prepared sources and resolves pinned network views and anchors.
118. `ResolveValuationInputsState` reads pinned views and resolves unit prices using valuation
     runtime adapters.
119. Each `ObserveCompiledBatchState` reads resolved subjects, pinned views, and valuations, then
     executes one compiled observation batch through the planned observation adapter.
120. `MergeObservationsState` concatenates and sorts all batch outputs into one `Vec<Observation>`.

### Snapshot assembly and artifact creation

121. `AssembleSnapshotState` reads:

    - resolved subjects
    - pinned views
    - merged observations

122. It groups observations by wallet.
123. It materializes `NetworkPin`s from the pinned execution anchors.
124. It materializes `WalletSnapshot`s from portfolio wallets plus resolved subject data.
125. It normalizes symbol configs.
126. It obtains `generated_at_ms` from `io.now_millis()`.
127. It constructs:

    ```rust
    PortfolioSnapshot {
        schema_version: 2,
        portfolio_id,
        generated_at_ms,
        network_pins,
        wallets,
        symbol_configs,
        errors: [],
    }
    ```

128. It normalizes the snapshot and writes the full JSON snapshot into context key:

    ```text
    portfolio_execute.main.out.snapshot
    ```

129. It then calls `write_output_artifact(...)`.
130. `write_output_artifact()`:

    - checks whether the fact key already exists
    - records the value through `io.record_value(fact_key, value)`
    - stores the returned artifact id in context key
      `portfolio_execute.main.out.snapshot_artifact_id`
    - emits `artifact_written` if this is the first binding for that fact key

131. That output artifact is the canonical persisted `PortfolioSnapshot` JSON artifact.

### Report projection and final run completion

132. `ProjectReportState` reads the snapshot JSON from context.
133. It derives wallet totals and portfolio totals by quote.
134. It constructs `PortfolioReport { schema_version: 2, ... }`.
135. It writes the report JSON to context key:

    ```text
    portfolio_execute.main.out.report
    ```

136. It emits a domain event named `portfolio_tracker.completed`.
137. After the final state succeeds, the engine appends:

    ```text
    RunCompleted { status, final_snapshot_id }
    ```

138. `final_snapshot_id` here is the final machine context snapshot artifact id, not the portfolio
     snapshot output artifact id.

### Final response extraction

139. `AppServices::start_portfolio_snapshot()` receives the completed run result.
140. If the run phase is `"failed"`, it scans the run stream backward for the most recent
     `StateFailed` event and maps that error to an `AppError`.
141. If the run completed and `final_snapshot_id` exists, it loads that artifact through
     `mfm_sdk::unstable::load_context_snapshot_json(...)`.
142. It reads, with typed slot-fallback decoding:

    - `portfolio_tracker.main.assemble_snapshot.out.snapshot_artifact_id`
    - `portfolio_tracker.main.project_report.out.report`

143. It deserializes those values and constructs:

    ```rust
    PortfolioSnapshotResponse {
        run_id,
        phase,
        final_snapshot_id,
        snapshot_artifact_id,
        report,
    }
    ```

### Final JSON envelope production and validation

145. The CLI command builds:

    ```rust
    FeatureExecutionResult {
        feature_id: "portfolio.snapshot",
        result: <PortfolioSnapshotResponse as JSON>,
    }
    ```

146. `handle_command_result()` sees `--output-format json` and wraps it as:

    ```json
    {
      "status": "success",
      "data": {
        "feature_id": "portfolio.snapshot",
        "result": {
          "run_id": "...",
          "phase": "completed",
          "final_snapshot_id": "...",
          "snapshot_artifact_id": "...",
          "report": { "...": "..." }
        }
      }
    }
    ```

147. The outer shell wrapper captures that stdout into `result_file`.
148. It validates only:

    - `.status == "success"`
    - `.data.feature_id == "portfolio.snapshot"`
    - `.data.result` exists

149. If validation passes, the wrapper runs `cat "$result_file"`, so the user receives the exact
     inner CLI stdout unchanged.
150. If validation fails, the wrapper prints an error, dumps the bad stdout to stderr, and exits
     non-zero.

## 4. File-level map

### Project-owned Nix entrypoint and wrapper

- [`nixfied/project/module.nix`](../nixfied/project/module.nix)
  - `mkCommandTask`
  - `mkTaskApp`
  - `resolve_packaged_mfm_cli_binary`
  - `task.mfm.portfolio.snapshot`
  - `apps."mfm::portfolio::snapshot"`

- [`nixfied/project/conf.nix`](../nixfied/project/conf.nix)
  - `services.postgres`
  - `services.helios`
  - `packages."mfm-cli"`

### Relevant framework files under `nixfied/framework/`

- [`nixfied/framework/core/mkFlakeOutputs.nix`](../nixfied/framework/core/mkFlakeOutputs.nix)
  - flake app launcher path for `nix run`

- [`nixfied/framework/launch/run-selected-app.nix`](../nixfied/framework/launch/run-selected-app.nix)
  - selected app resolution and `exec ${selectedApp.program}`

- [`nixfied/framework/core/materializeExecution.nix`](../nixfied/framework/core/materializeExecution.nix)
  - turns `taskRef` apps into runtime programs that call `nixfied-orchestrator run-task`

- [`nixfied/framework/runtime/orchestrator.nix`](../nixfied/framework/runtime/orchestrator.nix)
  - orchestrator binary and runtime env setup

- [`nixfied/framework/runtime/executor.nix`](../nixfied/framework/runtime/executor.nix)
  - `execute_task_once()` and shell runner dispatch

- [`nixfied/framework/core/mkServiceRuntimeSurfaces.nix`](../nixfied/framework/core/mkServiceRuntimeSurfaces.nix)
  - generates `serviceHookEnv`

- [`nixfied/framework/runtime/helpers/service-api.nix`](../nixfied/framework/runtime/helpers/service-api.nix)
  - maps service ops to env vars such as `SVC_POSTGRES_READY`

- [`nixfied/framework/runtime/helpers/service-policy.nix`](../nixfied/framework/runtime/helpers/service-policy.nix)
  - owner/discovery policy helpers

- [`nixfied/framework/runtime/helpers/skip-policy.nix`](../nixfied/framework/runtime/helpers/skip-policy.nix)
  - `is_service_skipped`

### Service lifecycle files

- [`nixfied/framework/runtime/services/postgres/default.nix`](../nixfied/framework/runtime/services/postgres/default.nix)
  - service op contract and hook mapping

- [`nixfied/framework/runtime/services/postgres/lifecycle.nix`](../nixfied/framework/runtime/services/postgres/lifecycle.nix)
  - runtime prelude
  - start/readiness/setup-db/full-start

- [`nixfied/framework/runtime/services/helios/default.nix`](../nixfied/framework/runtime/services/helios/default.nix)
  - service op contract and hook mapping

- [`nixfied/framework/runtime/services/helios/lifecycle.nix`](../nixfied/framework/runtime/services/helios/lifecycle.nix)
  - runtime prelude
  - config validation
  - checkpoint derivation
  - start command
  - readiness loop

### Rust CLI path

- [`bin/cli/src/commands/portfolio/snapshot.rs`](../bin/cli/src/commands/portfolio/snapshot.rs)
  - request parsing and response envelope construction

- [`bin/cli/src/support/app_services.rs`](../bin/cli/src/support/app_services.rs)
  - engine and app service wiring

- [`bin/cli/src/support/run_stores.rs`](../bin/cli/src/support/run_stores.rs)
  - stream store and artifact store construction

### App, planner, runtime, and domain execution

- [`crates/app/src/lib.rs`](../crates/app/src/lib.rs)
  - `AppBuilder`
  - `make_engine_bundle`
  - `AppServices`
  - `start_run`
  - `start_portfolio_snapshot`
  - `parse_portfolio_snapshot_request_input`
  - `FeatureCatalog`

- [`crates/portfolio-config/src/lib.rs`](../crates/portfolio-config/src/lib.rs)
  - authored JSON/TOML parsing
  - canonicalization into the typed portfolio request
  - deterministic build into the execution op's built config

- [`crates/ops/portfolio-tracker-op/src/lib.rs`](../crates/ops/portfolio-tracker-op/src/lib.rs)
  - legacy canonical vs built-config request decoding
  - child-op lowering
  - root export keys

- [`crates/ops/portfolio-tracker-op/src/plan_ops.rs`](../crates/ops/portfolio-tracker-op/src/plan_ops.rs)
  - internal child op ids
  - leaf state lowering
  - context slot wiring

- [`crates/states/portfolio/src/execution_states.rs`](../crates/states/portfolio/src/execution_states.rs)
  - actual semantic runtime state handlers

- [`crates/states/common/src/output.rs`](../crates/states/common/src/output.rs)
  - output artifact recording helper

- [`crates/sdk/src/unstable.rs`](../crates/sdk/src/unstable.rs)
  - pipeline planning
  - manifest persistence
  - compiled execution spec persistence
  - run launcher

- [`crates/machine/src/runtime.rs`](../crates/machine/src/runtime.rs)
  - engine start/resume
  - state loop
  - run completion

- [`crates/machine/src/runtime/attempt.rs`](../crates/machine/src/runtime/attempt.rs)
  - per-state attempt execution

- [`crates/machine/src/context_runtime.rs`](../crates/machine/src/context_runtime.rs)
  - context snapshot encoding/decoding

### Managed RPC and control-plane storage

- [`crates/collectors/rpc-control/src/lib.rs`](../crates/collectors/rpc-control/src/lib.rs)
  - typed request/response contracts
  - fact-key derivation
  - `EvmIoClient`

- [`crates/transports/rpc-control/src/lib.rs`](../crates/transports/rpc-control/src/lib.rs)
  - env bootstrap catalog parsing
  - control-plane store selection
  - `prepare_sources_impl`
  - managed source selection

- [`crates/storages/control-plane-postgres/src/lib.rs`](../crates/storages/control-plane-postgres/src/lib.rs)
  - `connect_env()`
  - control-plane schema initialization

## 5. Data flow

### Input request shape

The canonical top-level input is:

```json
{
  "portfolio": { "...": "PortfolioConfig" },
  "valuation_source_registry": { "...": "ValuationSourceRegistry" }
}
```

Important `PortfolioConfig` fields:

- `portfolio_id`
- `quote_codes`
- `networks`
- `wallets`
- `symbol_configs`
- `metadata`

Important `NetworkConfig` fields:

- `network_id`
- `family`
- `chain_id` for EVM
- `control_scope`
- `metadata`

`control_scope` defaults to `shared`.

### Important env vars

Outer wrapper/service orchestration:

- `SERVICE_OWNER_SCOPE`
- `SERVICE_DISCOVERY_SCOPE`
- `SKIP_HELIOS`
- `POSTGRES_PORT`
- `HELIOSRPC_PORT`
- `HELIOS_NETWORK`
- `HELIOS_EXECUTION_RPC_URL`
- `HELIOS_CONSENSUS_RPC_URL`
- `HELIOS_CHECKPOINT`
- `HELIOS_READY_TIMEOUT_SECS`
- `HELIOS_READY_INTERVAL_SECS`

Rust runtime and stores:

- `DATABASE_URL`
- `MFM_ARTIFACT_ROOT`

Managed RPC bootstrap:

- `MFM_EVM_RPC_SOURCES_JSON`
- `MFM_EVM_RPC_PREFERRED_ORDER`
- `MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS`

### Temporary files created

Task wrapper:

- `result_file` under `${TMPDIR:-/tmp}/mfm-portfolio-snapshot.XXXXXX.json`

Service-managed runtime files:

- Postgres:
  - `PGDATA/postgresql.conf`
  - `PGDATA/postmaster.pid`
  - `PGDATA/postgres.log`
- Helios:
  - `HELIOS_DIR/run/helios.pid`
  - `HELIOS_DIR/logs/helios.log`
  - `HELIOS_DIR/data/*`

[Inference] The exact absolute on-disk service directories are resolved from Nixfied slot/runtime
state at launch time, not hard-coded in the snapshot task body.

### Run ids, artifact ids, and snapshot ids

- `run_id`
  - UUID generated when the engine starts

- `manifest_id`
  - content-addressed artifact id for the run manifest
  - not returned by `portfolio snapshot`, but present in the run stream

- compiled execution spec artifact id
  - content-addressed artifact id for the optional compiled execution spec
  - written into initial context under `mfm.compiled_execution_spec_artifact_id`

- `final_snapshot_id`
  - content-addressed artifact id of the final machine context snapshot
  - returned in the final JSON result

- `snapshot_artifact_id`
  - content-addressed artifact id of the canonical `PortfolioSnapshot` output artifact
  - returned in the final JSON result

### Context keys written and read

Important writes during the flow:

- `mfm.compiled_execution_spec_artifact_id`
- `portfolio_execute.main.out.prepared_sources`
- `portfolio_execute.main.out.resolved_subjects`
- `portfolio_execute.main.out.pinned_views`
- `portfolio_execute.main.out.resolved_valuations`
- `portfolio_execute.main.out.observations`
- `portfolio_execute.main.out.snapshot`
- `portfolio_execute.main.out.snapshot_artifact_id`
- `portfolio_execute.main.out.report`

The final response extraction reads:

- `portfolio_execute.main.out.snapshot_artifact_id`
- `portfolio_execute.main.out.report`

from the final context snapshot artifact addressed by `final_snapshot_id`.

### Output JSON envelope shape

Success envelope:

```json
{
  "status": "success",
  "data": {
    "feature_id": "portfolio.snapshot",
    "result": {
      "run_id": "uuid",
      "phase": "completed",
      "final_snapshot_id": "artifact-id-of-context-snapshot",
      "snapshot_artifact_id": "artifact-id-of-portfolio-snapshot",
      "report": {
        "schema_version": 2,
        "portfolio_id": "...",
        "generated_at_ms": 0,
        "network_pins": [],
        "wallet_summaries": [],
        "totals_by_quote": [],
        "error_count": 0
      }
    }
  }
}
```

Error envelope for direct CLI JSON mode:

```json
{
  "status": "error",
  "error": {
    "code": "SomeCode",
    "message": "human-readable message"
  }
}
```

## 6. Failure points

### Shell / Nix / framework failures

These happen before or around Rust startup:

- unknown app id or selected launcher problems
- wrong arg count
- request file missing or unreadable
- missing `POSTGRES_PORT`
- missing Postgres lifecycle hooks
- invalid owner/discovery policy combination
- removed envs `MFM_KEEP_SERVICES` or `SERVICE_REUSE_POLICY`
- missing `HELIOSRPC_PORT` when Helios path is used
- `HELIOS_NETWORK != mainnet`
- missing `MFM_EVM_RPC_SOURCES_JSON` when Helios is skipped/unavailable
- packaged `mfm_cli` binary missing

User-visible shape:

- plain stderr from shell/framework code
- non-zero exit
- no validated JSON stdout envelope

### Service lifecycle failures

Postgres failures:

- runtime variable resolution failure
- config template missing
- Darwin shared memory preflight failure
- port conflict against another server
- `pg_ctl` startup failure
- readiness probe timeout
- database setup failure

Helios failures:

- binary missing
- missing execution RPC URL
- missing consensus RPC URL for non-local network
- checkpoint derivation failure
- startup failure
- readiness timeout

User-visible shape:

- plain stderr from wrapper/service script
- non-zero exit
- no validated JSON stdout envelope

### Rust CLI / app-layer failures

- invalid request-file contents
- invalid JSON
- missing `DATABASE_URL`
- Postgres stream-store connection failure
- invalid artifact backend
- transport registration failure
- request validation failure from `validate_portfolio_bundle`
- Aave-specific portfolio validation failure
- internal serialization failures

User-visible shape for direct CLI JSON mode:

- JSON error envelope on stderr
- non-zero exit

User-visible shape for `mfm::portfolio::snapshot` wrapper:

- because the wrapper runs the CLI under `set -e`, a non-zero CLI exit aborts the wrapper before
  stdout envelope validation
- the user sees CLI stderr and wrapper exit status

### Planner / machine / state execution failures

- invalid public op id or invalid op config
- unsatisfied planner imports/duplicate exports
- canonical JSON hashing failures
- manifest or compiled-spec storage corruption mismatch
- state failures with `StateFailed`
- context snapshot decode failures
- final report or artifact-id decode failures

When a state fails:

- the machine appends `StateFailed`
- run phase becomes `failed`
- `AppServices::start_portfolio_snapshot()` scans the run stream backward and converts the latest
  `StateFailed` payload into an `AppError`

### Managed RPC / control-plane failures

- invalid `MFM_EVM_RPC_SOURCES_JSON`
- sources missing `network_id`
- no sources configured for requested network
- catalog mismatch for an existing source pool
- no healthy sources after probing
- control-plane Postgres connection/init failure
- downstream RPC call failures

Typical error codes in this area include:

- `rpc_control_no_sources`
- `rpc_control_network_required`
- `rpc_control_catalog_mismatch`
- `rpc_control_config_invalid`
- `rpc_control_no_healthy_sources`

### How persistent local runtime state affects behavior

Persistent runtime state changes behavior in several places:

- service reuse
  - a ready Postgres or Helios instance may be reused
  - cleanup may be skipped when `SERVICE_OWNER_SCOPE=persistent`

- Postgres database existence
  - a reused server may not yet contain database `mfm`, so the wrapper always reruns setup-db

- control-plane ranking history
  - `rpc.control` source and pool state persist in Postgres across runs that share the same
    `DATABASE_URL`

- artifact reuse
  - old artifacts remain available in the filesystem artifact store because artifacts are
    content-addressed and immutable

## 7. Architecture assessment

### Why the layering exists

The flow is explicitly split so each layer owns one concern:

- Nixfied app/task/orchestrator layers own launch-time and service-lifecycle concerns.
- The Rust CLI owns transport parsing and stable output.
- `mfm-app` owns assembly of registries, transports, stores, and run entrypoints.
- The op layer owns deterministic planning only.
- The state layer owns runtime execution only.
- The machine runtime owns event-sourcing, checkpointing, and replay/resume semantics.

This aligns well with:

- `docs/architecture.md` thin-layer rules
- `docs/design.md` manifest/event/artifact invariants

### What is elegant about the design

There are several genuinely strong design choices here:

- The outer wrapper keeps operational concerns out of Rust domain code.
- The Rust CLI remains thin: parse -> call app services -> render result.
- `portfolio_config_build` and `portfolio_execute` are real planner ops, not transport glue.
- `portfolio_tracker` now composes the canonical-input build step inside the op layer instead of in
  `mfm-app`.
- `portfolio_execute` remains a real built-config execution op instead of a giant CLI command.
- Runtime logic is concentrated in reusable semantic states.
- The engine persists manifest, context snapshots, output artifacts, and kernel events in a clean
  event-sourced model.
- `rpc.control` centralizes managed EVM ingress instead of letting states choose raw URLs.

### What is fragile or surprising

A few parts are worth calling out:

- The direct CLI command returns a feature-shaped envelope but does not actually dispatch through
  `FeatureCatalog`. It manually rebuilds the same shape.
- The outer wrapper validates only a shallow `jq` shape, not the full output schema.
- `final_snapshot_id` sounds like the portfolio snapshot artifact, but it is actually the final
  machine context snapshot id.
- `snapshot_artifact_id` is the domain snapshot artifact id. The two ids are easy to confuse.
- `mfm::portfolio::snapshot` advertises Helios-backed snapshotting, but Helios is optional if the
  caller provides managed RPC sources.

### Where responsibilities are cleanly separated

Clean boundaries:

- service lifecycle vs Rust business logic
- CLI transport layer vs app service layer
- planner vs state execution
- machine/runtime correctness vs domain semantics
- managed RPC routing vs domain portfolio logic

### Where responsibilities are somewhat blurred

The main blurred boundary is output shaping:

- `FeatureCatalog` exists as the app-level built-in feature dispatch layer
- `portfolio snapshot` CLI does not use it directly
- the shell wrapper knows enough about the inner CLI JSON structure to validate `feature_id`

That is workable, but it creates more than one place where the same contract is represented.

### What I would refactor first

If I were changing this area, my first refactors would be:

1. Finish making `mfm_cli portfolio snapshot` dispatch through the same `FeatureCatalog` path used
   by `portfolio.snapshot`.
   Request parsing already goes through the shared app helper; the remaining duplicated boundary is
   the CLI-owned `FeatureExecutionResult` envelope construction.
2. Make the outer wrapper validate against a stronger schema-aware contract instead of only checking
   three `jq` predicates.
3. Consider naming the machine checkpoint id more explicitly in user-facing results, for example
   `final_context_snapshot_id`, to reduce confusion with `snapshot_artifact_id`.

## Mental model

`nix run .#mfm::portfolio::snapshot` is an operational shell wrapper around a packaged Rust
command.

The wrapper gets the runtime ready:

- choose policy
- start or reuse Postgres
- start or reuse Helios if available
- seed managed RPC bootstrap env
- construct `DATABASE_URL`

Then the Rust CLI does the real work:

- parse request
- build app services
- start `portfolio_tracker`
- let the op layer compose `portfolio_config_build` and the fixed semantic execution graph
- run the graph sequentially through the machine runtime
- persist machine checkpoints and output artifacts
- extract the report and snapshot artifact id from the final context snapshot
- print one stable JSON envelope

The wrapper finally sanity-checks that envelope and replays it unchanged to stdout.
