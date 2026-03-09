# Refactor Publish Docs

Updated: 2026-03-09
Status: Execution plan

## Purpose

- Replace the design-comparison memo with a committed implementation plan.
- Move `publish-docs` registry planning to an `index-first` model.
- Stop bursty crates.io API fanout during planning.
- Preserve safe, explainable publish behavior under remote uncertainty.

## Scope

- Focus on planning and observation behavior for `nix run .#publish-docs`.
- Focus on crates.io index visibility, crates.io API fallback, and docs.rs probing.
- Focus on UX/DX contracts: logs, artifacts, explainability, resumability, and umbrella flow.
- Keep publish upload mechanics (`cargo publish`, `cargo yank`) out of scope except where plan/apply semantics depend on them.

## Current Snapshot

### Current Failure Mode

- Current run artifact: `.mfm/publish-docs/runs/run_1773037674_372401000/`
- `registry.json` recorded `temporary_error` for every probed package.
- `plan.json` mapped every selected package to `action=manual_review reason=registry-temporary-error`.
- `results.json` completed no publishes and returned no resume hint.

### Current Technical Causes

- `CratesIoClient::observe_packages` spawns one task per package with `JoinSet`.
- `DocsRsClient::observe_packages` does the same.
- There is no concurrency cap, no pacing, no retry loop, and no `Retry-After` handling.
- Network and response failures are collapsed into coarse statuses.
- Selecting `mfm-docs` in plan/apply triggers full-catalog observation in `prepare_run`.

### Current Repo Facts

- Catalog size: `42` packages.
- `docs_policy = "docs-rs"` count: `16`.
- `mfm-publish-docs` does not initialize tracing today.
- Current JSON error output is printed to stderr; success output is printed to stdout.

## Committed Decisions

## 1. Registry Strategy: Index-First

The target architecture is `index-first`.

This tool will use the crates.io sparse index as the primary source of truth for:

- exact local version presence
- newer remote version existence
- selected local dependency visibility
- post-publish propagation / `wait_registry`

The crates.io HTTP API is not the primary planner source.

### Why

- Cargo dependency resolution is index-based.
- `wait_registry` is fundamentally about index visibility, not API health.
- The index is a better fit for publish/noop decisions than the crates.io crate API.
- This sharply reduces planner pressure against crates.io API endpoints.

## 2. Index Access Model

The tool will not depend on Cargo's internal cache layout as its primary contract.

The tool will maintain its own registry cache under:

- `.mfm/publish-docs/cache/registry-index/`

The tool will not shell out to Cargo to refresh registry metadata as the primary design.

The refresh path will be a self-contained sparse-index reader with bounded concurrency and host-scoped pacing.

### Rationale

- Avoid coupling planning correctness to `CARGO_HOME`.
- Avoid depending on Cargo cache layout details that are not this tool's contract.
- Keep Nix/dev/CI behavior explicit and portable.

## 3. crates.io API Role

The crates.io API remains available only for:

- fallback diagnostics
- narrow confirmation of anomalous states
- future operator tooling that is explicitly diagnostic

The API does not override a fresh index result for publish/noop decisions.

If index refresh or index interpretation is inconclusive, the planner fails closed using `wait_registry` or `manual_review`; it does not publish based on an API-only happy path.

## 4. Freshness Contract

Registry observation must carry explicit freshness and source metadata.

Planner decisions may only treat an index result as authoritative when the observation is `fresh`.

### Required observation fields

At minimum, the registry observation model must distinguish:

- `source`
  - `index`
  - `api`
- `freshness`
  - `fresh`
  - `cached`
  - `unavailable`
- `observed_at`
- sanitized diagnostic code/details

### Freshness semantics

- `fresh`: the package index record was fetched or revalidated during the current run.
- `cached`: a prior cached record exists, but the current run could not refresh it or deliberately skipped refresh.
- `unavailable`: neither a fresh nor cached usable record exists.

### Planner rule

- `fresh` index results may authorize `noop`, `publish`, `wait_dependencies`, `needs_version_bump`, or `wait_registry`.
- `cached` results may inform logs and artifacts, but they must not authorize a publish.
- `unavailable` results must fail closed.

## 5. Planner Semantics

The planner action contract is tightened as follows.

### `wait_registry`

`wait_registry` means:

- authoritative registry visibility is not yet sufficient to advance safely

This includes:

- post-publish propagation lag in the index
- retryable index refresh failures
- cached-only registry knowledge
- bounded backoff / rate-limit waiting

This action is resumable.

### `manual_review`

`manual_review` is reserved for non-retryable or contradictory states:

- auth failure
- malformed or contradictory remote data
- invalid index payload
- remote newer than local
- unexpected cache corruption
- unresolved contradiction between index and fallback diagnostics

This action is not automatically resumable.

### Stable reason families

The implementation should use stable reason strings in these groups:

- `wait_registry`
  - `registry-propagation-pending`
  - `registry-refresh-failed`
  - `registry-cached-only`
  - `registry-rate-limited`
- `manual_review`
  - `registry-auth-error`
  - `registry-invalid-response`
  - `remote-newer-than-local`
  - `contradictory-registry-state`
  - `registry-cache-corrupt`

## 6. stdout / stderr Contract

The target CLI contract is:

- `text` mode:
  - success output on stdout
  - error output on stderr
  - logs on stderr
- `json` mode:
  - exactly one machine-readable envelope on stdout for both success and error
  - logs on stderr only

This is the contract to implement and test during the refactor.

It intentionally differs from the current JSON-error-on-stderr behavior.

## 7. Umbrella Flow

Full-catalog remote observation is not part of normal publish gating.

### New rule

- `plan` / `apply` observe only the selected wave packages and any selected local dependencies needed for planning.
- `sync-umbrella` owns full-catalog registry/docs observation.
- Selecting `mfm-docs` in `plan` / `apply` must not automatically trigger full-catalog remote fanout.

### Publish gating for `mfm-docs`

Publishing `mfm-docs` requires a fresh umbrella sync state.

`sync-umbrella` will write a local sync artifact containing at least:

- current git commit or equivalent workspace revision marker
- catalog digest
- generated README digest
- registry/docs observation metadata used to render the README

`plan` / `apply` will use that sync artifact instead of doing live full-catalog remote observation.

If `mfm-docs` is selected and the sync artifact is missing or stale, the planner returns:

- `action=refresh_umbrella`

`apply` / `resume` should treat that action as an internal prerequisite:

- run the same full-catalog umbrella sync path automatically
- persist the refreshed sync artifact
- re-plan against the refreshed state

If that auto-sync changes `crates/docs/README.md` and publish actions still remain, stop with an explicit “commit or pass --allow-dirty” error rather than silently bypassing the dirty-tree policy.

This keeps normal publish gating small, makes umbrella fanout explicit, and still lets the default `nix run .#publish-docs` path self-heal when umbrella state is the only missing prerequisite.

## 8. Observability Reuse

Do not add a direct dependency on `mfm-app` just to reuse observability bootstrap.

Initial implementation should either:

- copy the minimal bootstrap locally into `publish-docs`, or
- extract a very small shared helper later if multiple small tools need it

Do not pull the full `mfm-app` dependency graph into this tool for logging alone.

## Implementation Plan

## Phase 0: Contract Lock

Before changing planner behavior:

- freeze the stdout/stderr contract described above
- define the tightened `wait_registry` and `manual_review` semantics
- add explicit source/freshness requirements to the observation model
- define the umbrella sync artifact contract

This phase is complete when the document and tests agree on these contracts.

## Phase 1: Visibility And Safety

Keep the current API-based registry observer temporarily, but harden the tool around it.

- initialize tracing in `mfm-publish-docs`
- emit logs to stderr only
- add top-level spans:
  - run_id
  - mode
  - wave
  - selection
  - observer strategy
- add per-request events:
  - host
  - package
  - attempt
  - response status
  - retry_after_sec
  - backoff_ms
  - outcome class
- add bounded concurrency
- add host-scoped pacing
- honor `Retry-After`
- map retryable uncertainty to `wait_registry`
- remove full-catalog observation from normal `plan` / `apply`
- make `sync-umbrella` the only full-catalog remote path

This phase reduces risk immediately while preserving a safe planner.

## Phase 2: RegistryObserver Abstraction

Introduce a source-agnostic registry observation layer.

### New abstraction

- `RegistryObserver`

### Implementations

- `IndexRegistryObserver` as primary
- `ApiRegistryObserver` as fallback / diagnostics only

Planner and artifact code must become source-aware without becoming source-specific.

## Phase 3: Index-First Cutover

Implement the sparse-index reader and switch planner decisions to it.

- add tool-managed sparse index cache
- refresh selected package records during each run
- record source/freshness/diagnostics in artifacts
- authorize publish/noop decisions only from fresh index observations
- keep API fallback narrow and non-authoritative

This phase completes the architectural move.

## Phase 4: Cleanup

After index-first behavior is stable:

- remove any API-first-only planner assumptions
- simplify fallback paths where they add little value
- tighten tests around propagation, cached-only states, and resumability

## Required Code Seams

These files are part of the implementation review set.

1. `crates/tools/publish-docs/Cargo.toml`
2. `crates/tools/publish-docs/src/main.rs`
3. `crates/tools/publish-docs/src/app.rs`
4. `crates/tools/publish-docs/src/apply.rs`
5. `crates/tools/publish-docs/src/umbrella.rs`
6. `crates/tools/publish-docs/src/remote/mod.rs`
7. `crates/tools/publish-docs/src/remote/crates_io.rs`
8. `crates/tools/publish-docs/src/remote/docs_rs.rs`
9. `crates/tools/publish-docs/src/model.rs`
10. `crates/tools/publish-docs/src/plan.rs`
11. `crates/tools/publish-docs/src/artifacts.rs`
12. `crates/tools/publish-docs/src/error.rs`
13. `crates/tools/publish-docs/tests/cli_integration.rs`

Potential new files:

- `crates/tools/publish-docs/src/remote/http.rs`
- `crates/tools/publish-docs/src/remote/index.rs`
- `crates/tools/publish-docs/src/remote/observer.rs`
- `crates/tools/publish-docs/tests/registry_observer_tests.rs`
- `crates/tools/publish-docs/tests/remote_backoff_tests.rs`
- `crates/tools/publish-docs/tests/umbrella_sync_contract_tests.rs`

## Test Matrix

### CLI Contract

- text success stays on stdout
- text errors stay on stderr
- json success is a single envelope on stdout
- json error is a single envelope on stdout
- logs never pollute JSON envelopes

### Registry Observation

- fresh index result with exact version present -> `noop`
- fresh index result with exact version absent and deps ready -> `publish`
- fresh index result with deps missing -> `wait_dependencies`
- fresh index result showing propagation lag -> `wait_registry`
- cached-only index result -> `wait_registry`
- index refresh failure with no cache -> `wait_registry`
- invalid index payload -> `manual_review`
- remote newer than local -> `manual_review`
- contradictory index and fallback diagnostics -> `manual_review`

### Retry / Backoff

- registry returns 429 with `Retry-After`
- registry returns repeated 5xx
- registry refresh times out
- docs.rs returns 429 or 5xx
- request pacing is host-scoped and bounded

### Umbrella

- selecting `mfm-docs` does not trigger full-catalog remote observation in `plan`
- `sync-umbrella` performs full-catalog observation
- stale or missing umbrella sync artifact -> `refresh_umbrella`
- fresh umbrella sync artifact allows `mfm-docs` planning without live full-catalog fanout

## Non-Goals

- Do not redesign `cargo publish` invocation in this refactor.
- Do not make crates.io API success the source of truth for publish safety.
- Do not depend on Cargo cache internals or `CARGO_HOME` as the primary registry contract.
- Do not let umbrella rendering reintroduce hidden full-catalog fanout into normal publish gating.

## Recommendation

Execute this as a staged refactor with an `index-first` destination.

The immediate goal is not "compare API-first vs index-first again."
The immediate goal is:

- lock the contracts
- harden observability and safety
- land the `RegistryObserver` seam
- cut planner authority over to a tool-managed sparse index cache

That gives the correct long-term architecture while keeping the transition reviewable and safe.
