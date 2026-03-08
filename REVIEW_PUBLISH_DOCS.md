# REVIEW_PUBLISH_DOCS

Generated: 2026-03-08

## Purpose

This document is a handover for redesigning `nix run .#publish-docs`.

It corrects the baseline description of the current task, narrows the first implementation to something reviewable, and separates near-term work from later extensions.

The goal is not to lock the repo into one oversized redesign. The goal is to define:

- what the task does today
- what the real gaps are
- what the first Rust replacement should do
- what should wait until later phases

## Current Baseline

Today `nix run .#publish-docs` already does more than a blind ordered `cargo publish` loop.

Current behavior:

- implemented as a Nix task in `nixfied/project/module.nix`
- reads `crates/docs/publish-wave.json`
- uses `cargo metadata --no-deps --format-version 1` to resolve local package versions
- checks crates.io indirectly by running `cargo search <pkg>`
- skips a package when the discovered remote version matches the local version
- errors when the discovered remote version appears newer than the local version
- publishes the remaining packages in catalog order with `cargo publish`
- supports:
  - `--dry-run`
  - `--allow-dirty`
  - `--from <package>`
  - `--only <package>`

Current limits:

- remote observation is coarse:
  - it only infers one remote version
  - it does not know all published versions
  - it does not know yanked versions
  - it does not know docs.rs status
- it does not persist plan or result artifacts
- it does not record release provenance
- it does not detect local changes against the last released source revision
- it does not regenerate `mfm-docs`
- it has no explicit blocked or resume model beyond "rerun the command"

`mfm-docs` also has one useful repo-specific property that should shape the redesign:

- `crates/docs/src/lib.rs` is a stable wrapper that includes `README.md`
- the landing page content is therefore effectively README-driven today
- umbrella generation should target `crates/docs/README.md` first, not both files by default

## Actual Problems To Solve

### 1. The publish wave is not a desired-state model

`crates/docs/publish-wave.json` is an execution list, not a policy catalog.

It answers:

- what order should this one release wave use?

It does not answer:

- which workspace crates are intended to be public at all
- which crates belong on the umbrella page
- which crates expect docs.rs
- which crates are retired
- which crates may be yanked

### 2. Remote observation is too weak

The current task uses `cargo search` plus shell parsing.

That is enough for a first wave, but not enough for reconciliation:

- it is not an exact version inventory
- it does not model yanks
- it does not model docs.rs lag or failure
- it relies on loose text parsing
- it compares versions with shell sorting instead of real semver parsing

### 3. There is no durable run state

The current task has no typed plan or result artifacts.

That makes these workflows harder than they should be:

- auditing why a package was skipped or published
- distinguishing blocked work from terminal failures
- resuming after rate limits or propagation delays
- scripting around the command output

### 4. There is no release provenance

The hard problem is not "is this version published".
The hard problem is "did local source change after the last published version was cut".

That requires provenance data, not just remote version existence.

### 5. `mfm-docs` is still manual

The current umbrella README explicitly expects follow-up edits after crates land on docs.rs.

That is workable for a one-off wave.
It is not a good long-term control surface.

## Constraints

These constraints should be treated as non-negotiable.

### crates.io and docs.rs

- published crate versions are immutable
- the normal update path is a new crate version, not a mutable docs refresh
- yanking is the lifecycle action; deletion is not the normal model
- docs.rs is downstream of crates.io and may lag independently

### Repo architecture and safety

- keep `nix run .#publish-docs` as the user-facing entry point
- move complex logic out of inline shell and into Rust
- do not persist secrets, registry tokens, or raw auth-sensitive output
- if structured run artifacts are written, they must contain only sanitized data
- planning logic must stay deterministic
- if `mfm-machine` is introduced later, live IO must remain explicit and compatible with the repo's "no ambient IO in state logic" rule

### Delivery constraints

- prefer small, reviewable phases
- do not block the Rust rewrite on solving every future release feature
- preserve wave scoping first; broaden to full-workspace reconciliation later

## End State

Eventually `nix run .#publish-docs` should be able to answer:

- what needs publishing now
- what is already up to date
- what is blocked on dependency visibility
- what changed locally but still needs a version bump
- whether `mfm-docs` is stale relative to published surface
- whether a package needs manual review or retirement handling

The end state should be:

- idempotent
- dependency-aware
- remote-state-aware
- scriptable
- safe to rerun after interruption

That does not mean every feature belongs in the first implementation.

## Recommended Rollout

### Phase 1: Replace The Shell Loop With A Rust Tool

This phase should replace the inline shell logic with a Rust crate while preserving the current release-wave operating model.

Recommended home:

- `crates/tools/publish-docs`

Phase 1 goals:

- keep `nix run .#publish-docs` as the entry point
- keep `crates/docs/publish-wave.json` as the selected release wave input
- replace shell parsing with typed Rust logic
- use `cargo metadata` for local package and dependency discovery
- use exact crates.io observation instead of `cargo search` text parsing
- use real semver parsing
- emit typed plan and result artifacts
- make reruns safe by turning already-published versions into `noop`

Phase 1 explicitly does not need:

- docs.rs probing
- `mfm-docs` generation
- committed release provenance ledger
- `needs_version_bump`
- yanking
- `mfm-machine`

This is the right first cut because it keeps the change local:

- same Nix app
- same wave input
- same basic publish semantics
- better correctness
- better artifacts

### Phase 2: Add Policy Catalog, Provenance, And Umbrella Generation

Once the Phase 1 planner and apply flow are stable, add the missing policy layer.

Phase 2 goals:

- introduce `crates/docs/catalog.toml` as the desired-state catalog
- keep wave scoping as a filter until all-public reconciliation is actually needed
- add release provenance so `needs_version_bump` becomes meaningful
- add docs.rs observation
- generate `crates/docs/README.md` from desired state plus observed remote state

Important rule:

- keep `crates/docs/src/lib.rs` static unless there is a real reason to change the wrapper
- generate the README first

### Phase 3: Add Resume Semantics And Lifecycle Controls

Only after the planner, policy model, and artifacts are stable should the design grow into a true reconciler.

Phase 3 candidates:

- explicit blocked-state resume
- rate-limit cooldown handling
- optional `mfm-machine` orchestration
- retirement and yank workflows
- broader full-workspace reconciliation beyond the current wave

Do not start here.

## Phase 1 Implementation Contract

This is the concrete contract for the first Rust replacement.

### Inputs

Phase 1 reads:

- `crates/docs/publish-wave.json`
- `cargo metadata --no-deps --format-version 1`
- crates.io remote state for selected packages

### Remote observation

Phase 1 should replace the current `cargo search` heuristic with exact package/version observation.

Minimum facts needed per package:

- package exists remotely
- exact local version exists remotely
- highest visible remote version

Recommended statuses:

- `absent`
- `present`
- `temporary_error`
- `auth_error`
- `invalid_response`

Rules:

- exact local version present: local action can be `noop`
- remote version lower than local: local action can be `publish`
- remote version equal to local: `noop`
- remote version higher than local: `manual_review`

Use real semver parsing.
Do not use string comparison or shell `sort -V`.

### Local dependency model

Derive dependencies from local workspace path dependencies visible in `cargo metadata`.

Phase 1 ordering rules:

- only consider packages selected by the current wave
- ignore external registry dependencies for ordering
- keep publish execution serial

### Phase 1 action set

Keep the first action taxonomy small and consistent.

- `noop`
  - exact local version already exists remotely
- `publish`
  - exact local version is absent remotely
  - remote is not newer than local
  - selected workspace dependencies needed for publish are already satisfied
- `wait_dependencies`
  - target version is absent remotely
  - one or more selected upstream workspace packages still need to publish or become visible
- `manual_review`
  - remote newer than local
  - auth or owner error
  - invalid local metadata
  - contradictory remote state

Phase 1 should not classify:

- `needs_version_bump`
- `wait_docs_rs`
- `refresh_umbrella`
- `yank`

Those depend on later phases.

### CLI surface

Keep the Nix entry point stable:

- `nix run .#publish-docs`

Recommended Rust CLI:

- `nix run .#publish-docs -- plan`
- `nix run .#publish-docs -- apply`

Compatibility behavior:

- `nix run .#publish-docs` defaults to `apply`
- `--dry-run` remains supported as an alias for `plan`
- preserve:
  - `--allow-dirty`
  - `--from <package>`
  - `--only <package>`
- add `--json` for machine-readable output

Do not add `resume`, `sync-umbrella`, or `yank` in Phase 1.

### Run artifacts

Phase 1 should write sanitized per-run artifacts under:

- `.mfm/publish-docs/runs/<run-id>/`

Recommended files:

- `catalog.json`
- `local.json`
- `remote.json`
- `plan.json`
- `results.json`
- `summary.json`

These artifacts should contain:

- normalized package metadata
- normalized remote status
- chosen action
- safe error categories

These artifacts should not contain:

- registry tokens
- raw credential-bearing environment
- raw unfiltered publish stderr if it can expose auth context

### Safe rerun model

Phase 1 does not need full checkpointed resume.

Safe rerun is enough:

- if a package was published in a previous partial run, rerunning should classify it as `noop`
- if a downstream package is still blocked on remote visibility, rerunning later should continue cleanly

That is already useful and much cheaper than full orchestration machinery.

### Phase 1 acceptance criteria

Phase 1 is complete when all of the following are true:

- rerunning `plan` without local or remote change produces the same actions
- rerunning `apply` after a partial publish is safe and does not republish existing versions
- exact local version presence is checked without `cargo search` parsing
- remote newer than local becomes `manual_review`
- selected workspace dependencies influence plan order and blocking
- `--json` output is stable enough for scripts
- run artifacts are written and contain only sanitized data

## Phase 2 Implementation Contract

This phase adds policy and provenance. It should not change the Phase 1 action meanings.

### Desired-state catalog

File:

- `crates/docs/catalog.toml`

Suggested V1 shape:

```toml
catalog_version = 1
umbrella_package = "mfm-docs"

[[package]]
name = "mfm-machine"
workspace_path = "crates/machine"
visibility = "public"            # public | private | retired
section = "engine_sdk"           # engine_sdk | core | states | ops | storages | collectors | transports | binaries_tooling
summary = "State-machine runtime, execution plans, events, and recovery contracts."

# optional
docs_policy = "docs-rs"          # docs-rs | repo-only | hidden
umbrella_policy = "when-published" # always | when-published | never
release_priority = 100
allow_yank = false
owners = []
notes = ""
```

Required package fields:

- `name`
- `workspace_path`
- `visibility`
- `section`
- `summary`

Optional fields with defaults:

- `docs_policy`
- `umbrella_policy`
- `release_priority`
- `allow_yank`
- `owners`
- `notes`

Recommended defaults:

- `docs_policy = "docs-rs"` for public library or proc-macro crates
- `docs_policy = "repo-only"` for public crates that are not intended to have docs.rs as the primary surface
- `umbrella_policy = "when-published"` for public crates
- `umbrella_policy = "never"` for non-public crates and for `mfm-docs`
- `release_priority = 100`
- `allow_yank = false`
- `owners = []`
- `notes = ""`

Validation rules:

- `catalog_version` must be supported
- `name` must be unique
- `workspace_path` must be unique
- `workspace_path` must exist
- `name` must match `cargo metadata`
- `summary` must be non-empty
- `docs_policy = "docs-rs"` requires a library or proc-macro target
- `umbrella_policy != "never"` requires `visibility = "public"`

Warnings, not errors:

- public crate missing `license`
- public crate missing `repository`
- public crate missing `readme`

### Release provenance

`needs_version_bump` should not exist until provenance exists.

If Phase 2 adds provenance, keep the model simple and internally consistent:

- immutable release facts go in a committed ledger
- mutable remote observations stay in run artifacts

Recommended immutable ledger:

- `crates/docs/releases.json`

Recommended shape:

```json
{
  "schema_version": 1,
  "packages": {
    "mfm-machine": [
      {
        "version": "0.1.0",
        "git_commit": "abcdef1234567890",
        "published_at": "2026-03-08T05:40:00Z",
        "registry": "crates-io"
      }
    ]
  }
}
```

Ledger rules:

- append-only by released version
- exactly one entry per `(package, version)`
- never rewrite historical release facts

Do not store "latest docs.rs status" in this ledger.
That field is mutable by definition and belongs in per-run observation artifacts instead.

### `needs_version_bump`

Only add this action after the ledger exists.

Recommended rule:

- find the latest released entry for the package
- diff `workspace_path` against the recorded `git_commit`
- if the package changed locally and the exact local version is already published, classify `needs_version_bump`

This should be path-scoped, not full-repo-scoped.

### docs.rs observation

Add docs.rs only in Phase 2.

Minimum statuses:

- `not_expected`
- `absent`
- `pending`
- `available`
- `failed`
- `temporary_error`

Keep the first implementation simple:

- probe the expected versioned docs page
- record safe status only
- do not trigger rebuilds automatically

### `mfm-docs` generation

Generate:

- `crates/docs/README.md`

Keep `crates/docs/src/lib.rs` static unless the wrapper needs to change.

Generation inputs:

- `catalog.toml`
- observed published set
- observed docs.rs status

Generation rules:

- summaries come from `catalog.toml`
- do not scrape README or rustdoc for summaries
- only include packages with `umbrella_policy != "never"`
- `when-published` crates show docs.rs links only when published
- unpublished or docs-pending crates may render as `pending`
- ordering must be deterministic

Phase 2 adds these actions:

- `needs_version_bump`
- `wait_docs_rs`
- `refresh_umbrella`

## Phase 3 Notes

`mfm-machine` may be a good fit later because release flows are long-running and interruption-prone.

It should not be treated as a Phase 1 requirement.

If added later, require all of the following:

- explicit live IO boundaries
- sanitized persisted artifacts
- no secret-bearing event or artifact payloads
- coarse workflow states, not one state per tiny publish step

Possible later phases:

1. `LoadCatalog`
2. `DiscoverWorkspace`
3. `ObserveCratesIo`
4. `ObserveDocsRs`
5. `BuildPlan`
6. `ExecutePublishableActions`
7. `WaitForRegistryPropagation`
8. `WaitForDocsRs`
9. `RegenerateUmbrella`
10. `PublishUmbrella`
11. `SummarizeResults`

That is a future design option, not the first implementation contract.

## Bottom Line

The current task is better than a blind publish loop, but still too weak for long-term reconciliation.

The right redesign is incremental:

1. replace the shell with a small Rust tool for the current wave model
2. add typed artifacts and exact remote observation
3. add desired-state policy, provenance, docs.rs, and umbrella generation
4. only then consider full resumable orchestration and lifecycle controls

That sequence keeps the change reviewable, aligns with the repo's architecture constraints, and avoids baking contradictions into the first implementation.
