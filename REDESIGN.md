# MFM - Redesign

## System Design requirements
- Append-only transactional
- Reproducibility: Nix philosophy applied at the architectural level of the whole system.
- Simplicity: KISS philosophy
- Composability & Reusability: less lines of code means less bugs and less security issue, we achieve it by having a better architecture.
- Auditability: everything must be trackable
- Every execution happens inside an state machine
- Its state machines all the way down (e.g., we can have a new state machine inside a state handler)


## Workspace Structure

```
mfm/
├── Cargo.toml          # workspace root, v0.1.29, resolver v2
├── crates/        
├──── collectors/         # EVM chains, Coingecko API, etc... 
├──── storages/           # ClickHouse, PostgreSQL, MinIO
├──── mfm_core/           # blockchain primitives, keystore, config, encryption primitives, wallets
├──── mfm_machine/        # generic async-ready state machine framework
├────── mfm_machine_derive/ # proc-macro for StateMetadata boilerplate (proc-macro lib, v0.1.0)
├──── ops/              # collection of all possible operations (collect->store->transform->reuse blockchain data)
├────── aave-tracker/   # track all AAVE big movements
├────── portfolio-tracker/   # track collection of wallets operations as portfolios
├────── portfolio-management/   # management a portfolio on-chain
├── bin/        
├──── mfm_cli/          # command-line interface
├──── rest_api/             # HTTP REST API
├── flake.nix           # nix build/dev environment
└── flake.lock
```
> Lets rename all these modules removing the prefix mfm_

### Dependency graph

```
{ mfm_cli, rest_api } -> ops -> { mfm_core -> mfm_machine -> mfm_machine_derive, collectors, storages }
```

## Naming and Packaging

The repo currently uses `mfm_*` crate names. If we want to remove the `mfm_` prefix:

- Avoid conflicts with Rust std crates (`core`, `alloc`, etc). For example, `core` is not a viable crate name.
- If we ever publish to crates.io, unprefixed names like `machine`, `keystore`, `storage` are likely to collide.

Proposal:
- Directory layout: keep clean folder names under `crates/` and `bin/`.
- Package names: keep a `mfm-` prefix for crates.io uniqueness (e.g. `mfm-machine`).
- Rust crate names (`extern crate`): only remove prefixes if it stays ergonomic and does not cause collisions.
  - This can be done per-crate with `[lib] name = "..."` when it is worth the churn.

## Core Concepts

This redesign treats MFM as a reproducible workflow runtime with a strict audit trail.

Definitions:

- `Operation`: a versioned unit of work (example: `portfolio-tracker`).
- `Run`: one execution of an `Operation` with explicit inputs.
- `Machine`: the state machine definition for an `Operation` (a sequence/DAG of `State`s).
- `State`: one step in a `Machine`.
- `Event`: an append-only record describing what happened (`RunStarted`, `StateSucceeded`, ...).
- `Artifact`: immutable by-content data produced/consumed by a run (JSON blobs, parquet, tx hashes, etc).
- `Collector`: reads from external systems (RPC, HTTP APIs) and produces artifacts.
- `Storage`: writes/reads artifacts and run metadata (event logs, indexes).

## Non-Negotiable Invariants

These are architectural invariants (not just implementation details):

- Append-only: runs produce an immutable event log; no in-place mutation of prior history.
- Transactional step commits: a state either commits its outputs (events + artifacts) atomically, or commits nothing.
- Explicit impurity: side effects must be isolated in explicitly tagged states (similar to today’s `APPLY_SIDE_EFFECT` / `IMPURE`).
- Reproducible by default:
  - A run’s *inputs* are explicit and hashable.
  - A run’s *outputs* are content-addressed artifacts.
  - Re-executing with the same inputs should produce the same artifacts (unless the operation explicitly opts into "live" / time-dependent collection).
- Auditability: every persisted artifact is attributable to a run + state + inputs digest.
- Library boundaries: `crates/*` must remain usable without `bin/*`. (CLI/REST are consumers only.)
- Secrets never leak: no logging/printing/persisting of passwords, mnemonics, private keys, decrypted buffers.

## Execution Model (State Machines All The Way Down)

The runtime executes *machines*, not ad-hoc code paths. Even "simple" actions can be expressed as a 1-3 state machine to preserve uniformity.

Run lifecycle:

1. `RunStarted` event (records operation id/version, input digest, environment metadata).
2. For each state:
   - `StateStarted`
   - state handler executes (can spawn sub-machines)
   - on success: commit `StateSucceeded` + artifacts + "context writes" (as a single atomic commit)
   - on failure:
     - commit `StateFailed` (with classified error) if failure is recoverable/meaningful to record
     - or abort run immediately for unrecoverable errors
3. `RunCompleted` or `RunFailed` event.

Nested machines:
- A state handler may run a sub-machine.
- The parent run records a `SubRunLinked` event and treats the sub-run’s outputs as artifacts (by id/hash), rather than copying data.

## Append-Only + Transactional: What It Means Concretely

To satisfy "append-only transactional" without making everything huge:

- The *source of truth* for a run is an append-only event stream.
- The "current context" is a derived view (a projection) of that event stream.
- Each state commit writes:
  - events: `StateSucceeded` (+ structured metadata)
  - artifacts: stored immutably (content addressed)
  - projection updates: indexes/materialized views for fast reads (optional)

Failure/recovery semantics:
- Recovery is based on persisted events (not in-memory clones).
- Rewind is selecting an earlier event cursor (revision), not cloning an in-memory map.
- Idempotency is achieved by addressing artifacts by content hash and by recording which side-effecting actions were already applied.

## Reproducibility Model (Nix-ish)

"Reproducible" here means: given the same *declared inputs* and the same *pinned environment*,
we can deterministically re-compute (or re-fetch from the artifact store) the same outputs.

Each run should record:

- Operation identity: `operation_id`, `operation_version`.
- Inputs digest:
  - parameters (serde)
  - relevant config files (content hash)
  - explicitly declared "anchors" (block number ranges, timestamps, etc)
- Environment digest (best effort, no secrets):
  - git commit (or "dirty" marker) for the workspace
  - `Cargo.lock` hash
  - `flake.lock` hash (when running under Nix)
- Collector mode:
  - `Live`: perform network calls; persist responses as artifacts for auditability.
  - `Replay`: do not perform network calls; only read previously persisted response artifacts.

Rules of thumb:

- Time and randomness are inputs:
  - if a state needs "now", the chosen timestamp must be recorded as an input/event.
  - if a state needs randomness, record a seed and use a deterministic RNG.
- Secrets are never part of the audit log:
  - record references/ids (e.g. "used keystore entry UUID") but not secret material.
- Caching is an optimization, not a correctness requirement:
  - the artifact store enables memoization by content hash.
  - skipping a state due to cache must still emit an event explaining what happened.

## Implications for `machine` (Based on Current State)

Today’s `mfm_machine` is a good sketch but has known limitations (sync handlers, tracker snapshot issues, unbounded history, `&'static str` tags/labels, `println!`, etc).

The redesigned `machine` requirements:

- Async-first handlers (no blocking the runtime):
  - `StateHandler` should be `async` (or return a Future) so RPC/HTTP collection is first-class.
- Durable tracking uses a revision cursor, not `Arc` cloning:
  - tracking must store a stable "revision" (or serialized snapshot) so recovery is correct.
- Context should be append-only at the API boundary:
  - "write" should mean "append a patch" or "commit a batch", not mutate shared state.
- Bounded memory:
  - avoid O(n^2) history; keep history in the event log and optionally keep a bounded in-memory cache.
- No stdout/stderr in library crates:
  - remove `println!`; use `log` where needed, but default to silence.
- Tags/labels should support runtime values:
  - prefer `Cow<'static, str>` / `Arc<str>` + validation to support config-driven workflows.
- Dependency strategy must either work or be removed:
  - if we keep `DependencyStrategy`, it must be used by the scheduler/error handler in a test-backed way.

## Storage Split (Metadata vs Artifacts)

Model storage as two interfaces so we can mix and match implementations:

- `EventStore` (metadata):
  - append events, stream events, query by run/state, store run indexes
  - good fits: PostgreSQL / sqlite (dev) / other relational stores
- `ArtifactStore` (data):
  - put/get by content hash, optional pinning/GC, streaming reads/writes
  - good fits: filesystem (dev) / S3/MinIO / object stores

Analytics stores (ClickHouse) are consumers of events/artifacts:
- They are fed by machines or background replayers.
- They are not the source of truth for the audit log.

## Collectors

Collectors should be extremely small, deterministic adapters.

Guidelines:
- Inputs and outputs must be serializable and hashable.
- A collector may do network I/O, but it must:
  - expose explicit request parameters
  - return structured errors (retryable vs fatal)
  - never embed secrets in error messages
- Caching is handled by the runtime via artifact hashes, not by ad-hoc global caches.

## Operations (`ops/*`)

Each `ops/*` crate is a package of:

- Operation metadata: `id`, `version`, supported networks, feature flags.
- A machine definition: states + dependencies + tags.
- Types for inputs/outputs (serde-friendly).
- Optional domain-specific projections (e.g. "portfolio view") that can be derived from artifacts.

Operations must be testable without the CLI:
- unit tests for state logic
- integration tests for run event logs and recovery behavior

## Frontends (`bin/*`)

`mfm_cli` and `rest_api` are thin adapters over a shared runtime API.

- CLI must preserve stable text/JSON outputs (treat as public API).
- REST should return stable, machine-readable JSON with explicit error codes.
- Both must support non-interactive workflows (stdin/env vars, idempotent requests).

## Reuse Plan (What We Keep)

From the current codebase:

- Keep and evolve the keystore (security-hardened, comprehensive tests).
  - Near-term improvements: atomic writes (temp + rename), optional audit log, export/password rotation with explicit UX.
- Keep the state-machine idea, but refactor around the redesigned requirements above (async, durable tracking, append-only context).
- Keep the CLI contract style (scriptable, stable JSON output), but rebase it on the runtime execution model.

## Milestones (Small, Reviewable Steps)

1. Establish the new workspace layout (`crates/`, `bin/`) without behavior changes.
2. Introduce `EventStore` + `ArtifactStore` traits with a minimal filesystem/sqlite dev impl.
3. Add a `runtime` crate that can execute a trivial operation as a machine and persist an event log.
4. Refactor `machine` to async + revision-based tracking (test-backed).
5. Add one real operation end-to-end (collector -> artifacts -> projection), wire to CLI/REST.

Each milestone should remain shippable and keep crate boundaries intact.
