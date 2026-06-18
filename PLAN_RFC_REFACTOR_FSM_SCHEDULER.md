# Plan: Implement RFC_REFACTOR_FSM_SCHEDULER

Status: completed and validated

Completion notes, 2026-06-18:

- Scheduler drive paths load `VerifiedRunContext` through `VerifiedRunContextLoader`, which combines
  `BoundRuntimeContext` authority with store-owned committed stream/view authority.
- Runtime resume now compares the live bound runner executable identities against the `RunStarted`
  executable evidence admitted at run start. A mismatch fails before any resumed attempt starts.
- `ProjectionSnapshot::validate_run_stream` is the centralized ingress guard for old lifecycle
  streams. Runtime, replay, and Postgres projection rebuild/load paths reject attempt-bound terminal
  payloads that are not preceded by a separate `StateAttemptStarted` commit.
- `TransitionDecision` is explicit for started/continued/interrupted attempts, remediation starts,
  manual waits, saga terminal resolution, blocked status, and public-output completion.
- Framework attempts that have already started terminalize safe observed post-start failures,
  including output planning and artifact staging failures, into redacted diagnostic evidence and
  non-retryable `StateAttemptFailed` records.
- Saga terminal proofs now bind the exact prefix `next_seq`; `ResolveSagaTerminal` mints proof from
  the current post-start prefix, and store validation rejects stale proof/request prefixes.
- PostgreSQL `projection_snapshot()` now rebuilds the selected run snapshot from authoritative event
  rows and rebuilds active global resource lanes from authoritative run streams instead of trusting
  persisted projection rows.
- The async attempt path checks already-held resource lanes before staging runner artifacts, matching
  the sync path for known-blocked lanes.
- Local validation at this head:
  - `cargo fmt --all -- --check`
  - `cargo check --workspace`
  - `cargo test -p mfm-runtime`
  - `cargo test -p mfm-store --test commit_contract`
  - `cargo test -p mfm-replay`
  - `cargo test -p mfm-stream-store-postgres --all-features --no-run`
- `nix run .#ci` evidence:
  - Passed on 2026-06-18 with isolated state:
    `NIXFIED_STATE_DIR=/tmp/mfm-nixfied-ci.UkFW4I`.
  - Run id: `run-806297-1781792642754687448`.
  - Passed nodes: `ci.check.fmt`, `ci.check.clippy`, `ci.check.cargo-metadata-contract`,
    `ci.check.architecture-namespace-contract`, `ci.test.workspace-tests.nextest-run`,
    `ci.test.workspace-tests.doc-tests`, `ci.parity-cli-keystore`,
    `ci.test-db.postgres-sqlx-check`, `ci.test-db.parity-postgres-state-events`,
    `ci.test-db.parity-postgres-rest-api`, `ci.parity-reth-contracts`, and
    `ci.parity-reth-portfolio`.
  - The managed Postgres and Reth parity suites are therefore evidenced by the Nix CI run; no
    external `DATABASE_URL` was required in this shell.

This plan divides `RFC_REFACTOR_FSM_SCHEDULER.md` into reviewable commits for this branch. The
branch is allowed to make breaking changes. Do not add compatibility shims for old persisted streams,
old projection rows, old public response shapes, or mixed-version readers. If a stream or database was
written with the old lifecycle/event model, the new runtime should reject it with a clear typed
diagnostic unless a later explicit migration plan is requested.

`BootstrapRun` remains certified in this RFC because the RFC scopes removing it as a separate
certification/spec migration. Breaking changes are allowed, but they should still stay within this
RFC's boundary.

## Working Rules

- Keep each commit focused on one authority boundary or one schema cut.
- Update `docs/design.md`, `docs/architecture.md`, `docs/saga.md`, crate READMEs, CLI docs, and REST
  docs in the same commit that changes their contracts.
- Add tests that reject invalid shapes, not only tests that prove the new happy path works.
- Do not preserve old behavior through fragile compatibility code. Remove or rewrite old tests instead
  of keeping dual semantics.
- If an architectural question is unclear, spawn an architect-agent to resolve it against the RFC and
  this plan. If the question remains unresolved, stop and ask before coding.
- Use focused Cargo checks during each commit. Run `nix run .#check` and `nix run .#test` at the major
  semantic cuts and before handing off the branch. Run `nix run .#ci` only for final confidence or when
  managed Postgres/Reth parity is directly affected and focused checks are insufficient.

## Commit 1: `docs: align scheduler lifecycle contracts`

Scope:

- Add a high-level direction note to `docs/design.md` (and `docs/architecture.md` where it sets
  altitude) describing the target lifecycle model and the compatibility policy. Do not pre-write the
  detailed contract changes here — per the Working Rules, those land in the same commit as the code
  that changes them (frontier set in Commit 4, event schema/status in Commit 5, framework order in
  Commit 9, saga authority in Commits 11–12).
- State that this branch intentionally breaks old run streams, old projections, and public status
  shapes, and rejects old-model streams with a typed diagnostic. Mixed-version readers are
  unsupported.
- Keep the `BootstrapRun` certification migration explicitly out of scope.

Verification:

- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- `nix run .#check`

## Commit 2: `tests: pin scheduler refactor guardrails`

Scope:

- Add or rewrite baseline tests around authority boundaries that must survive the refactor:
  admission failure appends no semantic events, missing bindings fail before attempts, lane-blocked
  nodes preserve the scoped independence witness, and runners cannot emit lifecycle/store authority.
- Add negative tests for old-stream rejection where useful, instead of replaying old streams
  unchanged.
- Pin the optimistic-concurrency invariant: a commit carries the expected next stream sequence as a
  precondition, and a precondition conflict triggers reload + re-decide rather than failure (the
  resource-lane retry loop is one instance of this general pattern). Liveness is never inferred from
  the stream.
- Capture no-secret expectations for terminal failures, diagnostic artifacts, public output, and
  status surfaces.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`
- `cargo test -p mfm-app`

## Commit 3: `runtime: extract run admission and binding`

Scope:

- Add `admission.rs` and `binding.rs`.
- Move genesis preparation/append authority behind `RunAdmissionLifecycle`.
- Add `VerifiedRunContextLoader` and `BoundRuntimeContextLoader`.
- Make `BoundRuntimeContext` prove runner/capability/framework binding existence and identity only;
  live transport/signer/capability execution remains attempt-time behavior.
- Keep `BootstrapRun` certified and preserve the current genesis event batch shape.
- Move app assembly to build/load the bound context before transition or attempt execution.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-app`

## Commit 4: `runtime: split transition decision from dispatch`

Scope:

- Add `transition.rs` with closed `TransitionDecision`, and `history.rs` for verified history/view
  construction consumed by the pure decision.
- Keep `FrontierScheduler::decide` pure: certified runtime spec, verified history/view, and pure hints
  only. Do not pass stores, artifact stores, registries, runners, capabilities, transports, or signers.
- Include `node_id` and `attempt_id` in `ContinueAttempt`.
- Make saga remediation, manual wait, saga terminal resolution, and blocked outcomes explicit.
- Introduce the saga projection step as an explicit derivation: engagement, quiescence, owed
  obligations, manual block, and terminal-proof availability from certified spec plus verified
  history. Saga routing flows through this projection, not by mutating run mode.
- Preserve serial execution with `NoOpenAttempt` plus the existing resource-lane-scoped independence
  witness; do not introduce global open-attempt exclusion.
- Update the `docs/design.md` frontier description: the decision set is no longer exactly
  "run / block / complete".

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`
- test that an independent ready node still advances while another node is parked on a cross-run
  resource lane

## Commit 5: `events: cut attempt interruption schema`

Scope:

- Add `StateAttemptInterrupted` to `mfm-events`.
- Add `AttemptStatus::Interrupted` to store projections.
- Define interruption as retryable by construction; do not add a `retryable` field to the event.
- Enforce that standalone interruption is legal only for pure/read attempts and side-effect attempts
  with no acquired lane and no open ledger.
- Update store admission, projection rebuild, replay models, event codec helpers, app status models,
  CLI status, and REST status in one breaking cut.
- Scope status work here to the wire/projection layer: add the attempt-disposition field to the store
  projection and status models. The full public response shape and contract tests land in Commit 13;
  do not leave the JSON contract half-defined between the two.
- Remove old-stream compatibility expectations; readers should fail clearly on unsupported old
  schema/order when they cannot rebuild new authority.
- Centralize rejection in one ingress/load stream-model guard that emits the typed diagnostic, rather
  than scattering per-reader tolerance branches.
- Audit `Option`-typed event/projection fields and make required any field that was optional only to
  tolerate absence in older data (confirm per field; do not assume every `Option` is compat-driven).
- Update the `docs/design.md` and `docs/saga.md` event-schema and status sections in this same slice,
  including the attempt-disposition vs `RunMode` distinction.

Verification:

- `cargo test -p mfm-events`
- `cargo test -p mfm-store --test commit_contract`
- `cargo test -p mfm-replay`
- `cargo test -p mfm-app`
- test that `StateAttemptInterrupted` does not engage saga remediation and proves no AC/DC terminal
  outcome
- targeted CLI/REST status tests
- `nix run .#check`

## Commit 6: `postgres: rebuild projections for new attempt schema`

Scope:

- Update `stream-store-postgres` codec and projection storage for interrupted attempts.
- Remove old projection compatibility paths. Projection tables are rebuildable indexes for the new
  stream model.
- Keep rebuild single-path: do not branch on "with/without the new event"; rebuild assumes the new
  stream model and rejects anything else.
- Add schema drift and codec/projection round-trip tests for the new event.
- Make unsupported old stream rows fail fast with a typed diagnostic.

Verification:

- `cargo test -p mfm-stream-store-postgres`
- focused Postgres parity/schema checks with explicit `DATABASE_URL` when required
- `nix run .#test-db` if the change touches managed Postgres behavior beyond local unit coverage

## Commit 7: `runtime: add attempt lifecycle typestates`

Scope:

- Add `attempt.rs` with selected, started, invoked, terminal-planned, and terminal-committed phases.
- Route ordinary state attempts through `AttemptLifecycle` with no event-stream change yet.
- Keep input/config/fact materialization *before* the `StateAttemptStarted` commit, matching today: a
  materialization failure must still produce no started event. Reordering materialization to after the
  start commit is an event-semantics change deferred to Commit 8, where the failure-safe path can
  capture post-start materialization failures.
- Introduce `invocation.rs` (`InvocationBuilder`) for sealed runner-context construction, still
  invoked before the start commit in this commit.
- Keep runner contexts sealed and without store or artifact-store mutation handles.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`

## Commit 8: `runtime: terminalize observed attempt failures`

Scope:

- Add failure-safe terminalization from minimal trusted attempt authority. Add `runner_output.rs`
  (proposal validation), `commit.rs` (prepared commit builders), and `artifacts.rs` (staging helpers)
  as the supporting module cuts.
- Move input/config/fact materialization to after `StateAttemptStarted` (the event-semantics change
  deferred from Commit 7), so post-start materialization failures terminalize through the failure-safe
  path. This is a deliberate stream-content change for failing materializations; land it with its
  replay/projection/golden updates.
- Convert post-start materialization errors, handler errors, invalid runner output, artifact binding
  mismatches, public-output render failures, and retention projection failures into redacted terminal
  attempt evidence when storage is available.
- Derive `retryable` from certified policy and runtime failure classification, never from runner
  output.
- Keep authority, corruption, deployment, and storage outage failures outside semantic run events.
- Add diagnostic artifact handling without persisting secrets or bearer mutation material.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`
- no-secret/redaction tests for failure evidence
- `nix run .#check`

## Commit 9: `framework: route lifecycle nodes through attempts`

Scope:

- Route `PublicOutputRender`, `ProjectRetentionManifest`, `CompleteRun`, and `ResolveSagaTerminal`
  through the same post-admission start/run/terminal lifecycle in `framework.rs` (handlers only, no
  scheduling).
- Replace validators that require same-commit framework start/terminal ordering.
- Keep replay/projection on a single event-order model: do not add a reader that accepts both the old
  single-commit order and the new started-before-run order.
- Define recovery for open framework attempts by rebuilding output, manifests, and terminal proofs from
  current verified history.
- Keep `BootstrapRun` under run admission only.
- Update replay, public-output, retention, completion, and saga terminal tests for the new event order.
- Update the `docs/design.md` framework-lifecycle description for the new started-before-run and
  terminal event order.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-replay`
- `cargo test -p mfm-app`
- relevant CLI/REST status tests
- `nix run .#test`

## Commit 10: `recovery: own open attempt disposition`

Scope:

- Add `recovery.rs` and `AttemptRecoveryLifecycle`.
- On resume, classify open attempts from verified history before ordinary transition dispatch.
- Implement legal outcomes: continue, retry terminalization, interrupt, operational block, or delegate
  to side-effect recovery.
- Keep operational manual recovery separate from saga manual resolution.
- Preserve the resource-lane-scoped independence witness from transition planning.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`
- recovery-focused integration tests

## Commit 11: `side-effects: extract uncertainty lifecycle`

Scope:

- Add `side_effect_lifecycle.rs`.
- Keep store ledger typestate as the transition authority.
- Reject generic interruption/failure once a side-effect attempt has acquired a lane or open ledger,
  unless side-effect recovery records terminal evidence that releases the lane.
- Enforce post-`InvocationStarted` recovery outcomes: not-submitted proof, recovered submission/
  receipt/confirmation, or ambiguity paired with non-retryable attempt failure.
- Add forward-fence tests asserting store admission rejects new forward `InvocationStarted` after saga
  engagement.
- Update the `docs/design.md` / `docs/saga.md` side-effect authority sections, including the
  forward-fence authority note (store is the source of truth; runtime early-reject is convenience).

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`
- side-effect recovery tests
- `nix run .#test`

## Commit 12: `saga: rebuild terminal proof authority`

Scope:

- Remove scheduler-level `manual_terminal_proofs` caching.
- Rebuild and re-verify manual-resolution proof authority from the current verified prefix immediately
  before terminalization.
- Ensure `ResolveSagaTerminal` and `CompleteRun` terminal proofs cannot be stale.
- Update `mfm-manual-auth`, runtime, store, replay, and saga docs/tests as needed.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`
- `cargo test -p mfm-replay`
- manual-resolution and saga terminal proof tests

## Commit 13: `public-api: expose attempt disposition status`

Scope:

- Update app, CLI, and REST responses to expose attempt disposition separately from `RunMode`.
- Document the breaking JSON shape in `bin/cli/README.md` and `bin/rest-api/README.md`.
- Add contract tests for interrupted, open, failed, completed, resource-lane-blocked, manual-blocked,
  and terminal saga statuses.
- Ensure public output remains render-only and cannot become resume/replay authority.

Verification:

- `cargo test -p mfm-app`
- targeted CLI tests
- targeted REST tests
- `nix run .#check`

## Commit 14: `runtime: reduce serial scheduler facade`

Scope:

- Reduce `SerialTypedScheduler` to thin orchestration over admission/loading, transition dispatch,
  attempt lifecycle, recovery lifecycle, side-effect lifecycle, and commit planning.
- Delete duplicated sync/async scheduler branches where lifecycle authority logic is shared. Keep the
  thin sync/async driver seam (loads stream, awaits runners, stages artifacts, appends commits) per
  the RFC's First-Cut Decision; collapsing it is not in scope.
- Remove obsolete helper APIs, compatibility branches, and stale tests.
- Sweep for leftover compat scaffolding: tolerance branches, migration helpers, staged-rollout gates,
  and dual old/new goldens. Confirm no dual-version reader remains and that old goldens were replaced,
  not kept alongside new ones.
- Update `crates/kernel/runtime/README.md` module descriptions.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`
- `cargo test -p mfm-replay`
- `cargo test -p mfm-app`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- `nix run .#check`
- `nix run .#test`

## Commit 15: `test: run final scheduler refactor gate`

Scope:

- Run the complete planned verification gate.
- Fix any failures in the smallest owned surface; do not paper over failures with test relaxations.
- Run `nix run .#ci` only if final confidence requires managed Postgres/Reth parity, or if prior
  focused checks touched those services and left coverage gaps.
- Record any remaining unsupported old-stream behavior as intentional in docs/tests.

Verification:

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo test --workspace`
- `nix run .#check`
- `nix run .#test`
- `nix run .#ci` when justified by the criteria above
