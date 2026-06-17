# Plan: Implement RFC_REFACTOR_FSM_SCHEDULER

Status: implementation plan

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

- Update `docs/design.md`, `docs/architecture.md`, `docs/saga.md`, and runtime/app docs to describe
  the target lifecycle model before code moves.
- State that this branch intentionally breaks old run streams and public status shapes.
- Document that mixed-version readers are unsupported after the event-schema cut.
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

- Add `transition.rs` with closed `TransitionDecision`.
- Keep `FrontierScheduler::decide` pure: certified runtime spec, verified history/view, and pure hints
  only. Do not pass stores, artifact stores, registries, runners, capabilities, transports, or signers.
- Include `node_id` and `attempt_id` in `ContinueAttempt`.
- Make saga remediation, manual wait, saga terminal resolution, and blocked outcomes explicit.
- Preserve serial execution with `NoOpenAttempt` plus the existing resource-lane-scoped independence
  witness; do not introduce global open-attempt exclusion.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`

## Commit 5: `events: cut attempt interruption schema`

Scope:

- Add `StateAttemptInterrupted` to `mfm-events`.
- Add `AttemptStatus::Interrupted` to store projections.
- Define interruption as retryable by construction; do not add a `retryable` field to the event.
- Enforce that standalone interruption is legal only for pure/read attempts and side-effect attempts
  with no acquired lane and no open ledger.
- Update store admission, projection rebuild, replay models, event codec helpers, app status models,
  CLI status, and REST status in one breaking cut.
- Remove old-stream compatibility expectations; readers should fail clearly on unsupported old
  schema/order when they cannot rebuild new authority.

Verification:

- `cargo test -p mfm-events`
- `cargo test -p mfm-store --test commit_contract`
- `cargo test -p mfm-replay`
- `cargo test -p mfm-app`
- targeted CLI/REST status tests
- `nix run .#check`

## Commit 6: `postgres: rebuild projections for new attempt schema`

Scope:

- Update `stream-store-postgres` codec and projection storage for interrupted attempts.
- Remove old projection compatibility paths. Projection tables are rebuildable indexes for the new
  stream model.
- Add schema drift and codec/projection round-trip tests for the new event.
- Make unsupported old stream rows fail fast with a typed diagnostic.

Verification:

- `cargo test -p mfm-stream-store-postgres`
- focused Postgres parity/schema checks with explicit `DATABASE_URL` when required
- `nix run .#test-db` if the change touches managed Postgres behavior beyond local unit coverage

## Commit 7: `runtime: add attempt lifecycle typestates`

Scope:

- Add `attempt.rs` with selected, started, invoked, terminal-planned, and terminal-committed phases.
- Route ordinary state attempts through `AttemptLifecycle`.
- Commit `StateAttemptStarted` before materialization and runner execution.
- Move input/config/fact materialization into `InvocationBuilder` after attempt start.
- Keep runner contexts sealed and without store or artifact-store mutation handles.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`

## Commit 8: `runtime: terminalize observed attempt failures`

Scope:

- Add failure-safe terminalization from minimal trusted attempt authority.
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
  through the same post-admission start/run/terminal lifecycle.
- Replace validators that require same-commit framework start/terminal ordering.
- Define recovery for open framework attempts by rebuilding output, manifests, and terminal proofs from
  current verified history.
- Keep `BootstrapRun` under run admission only.
- Update replay, public-output, retention, completion, and saga terminal tests for the new event order.

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
- Delete duplicated sync/async scheduler branches where lifecycle authority logic is shared.
- Remove obsolete helper APIs, compatibility branches, and stale tests.
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
