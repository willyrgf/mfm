# Reuse Nixfied Handoff

## Purpose

Capture the current state of `mfm::portfolio::snapshot`, why the last fix landed in
`nixfied/project/module.nix`, and how to refactor this toward framework reuse in a
series of small commits.

This is a handoff for turning the current app-specific service orchestration into
framework-level composition.

## Current Working Repro

This now works:

```bash
NIX_ENV=0 SERVICE_REUSE_POLICY=same-slot nix run .#mfm::portfolio::snapshot -- 0x53078a4d5618f98123CFcFCdF5c20cb2Cdf97312
```

Observed result:

- `status: success`
- `feature_id: portfolio.snapshot`
- `phase: completed`
- `chain_id: 1`
- `block_number: 24620066`

Treat the block number as an example from one successful run, not a stable assertion.

This matters because the main bug was not "snapshot fails absolutely"; it was
"same-slot reuse does not survive the task/runtime model cleanly".

## What The Snapshot App Does Today

The app is defined in `nixfied/project/module.nix` as:

- task id: `task.mfm.portfolio.snapshot`
- app name: `mfm::portfolio::snapshot`

The task currently does much more than thin app wiring:

- validates args
- enforces `HELIOS_NETWORK=mainnet`
- computes service roots
- initializes/reuses Postgres
- probes listeners with `lsof`
- starts/reuses Helios
- manages pid files directly
- waits for framework `health` / `ready`
- applies an additional Helios mainnet sync gate
- runs `mfm_cli --output-format json portfolio snapshot`
- validates the final stdout payload is JSON before emitting it

That means the project wrapper is currently acting like a small service supervisor.

## Framework Pieces That Already Exist

The framework already provides most of the raw pieces:

- service policy helpers:
  - `nixfied/.framework/lib/service-policy.nix`
- generic background-service helper with policy-aware cleanup decisions:
  - `nixfied/.framework/lib/helpers.nix`
- generic service API / `svc::<service>::<op>` app generation:
  - `nixfied/.framework/lib/service-api.nix`
- Postgres lifecycle:
  - `nixfied/.framework/postgres/default.nix`
  - `nixfied/.framework/postgres/lifecycle.nix`
- Helios lifecycle:
  - `nixfied/.framework/helios/default.nix`
  - `nixfied/.framework/helios/lifecycle.nix`
- framework health/ready operations:
  - `nixfied/modules/operations.nix`
- detached run launching in the orchestrator:
  - `nixfied/runner/orchestrator.nix`
- detached `process-compose` supervisor lifecycle:
  - `nixfied/.framework/supervisor/default.nix`
  - `nixfied/.framework/supervisor/lifecycle.nix`

Important detail: Postgres already has a framework-level fix for short Unix socket
paths on macOS:

- `nixfied/.framework/postgres/lifecycle.nix`

Important detail: Helios already has framework readiness logic around `eth_blockNumber`
and pid-file-aware waiting:

- `nixfied/.framework/helios/lifecycle.nix`

Important detail: these are real building blocks, but they are not yet a drop-in
replacement for the snapshot wrapper:

- MFM does not currently configure `project.supervisor.services`
- there is no public `svc::<service>::ensure` or policy-aware generated
  `start-or-reuse` surface today

## Why The Last Project-Layer Fix Was Still Necessary

The confusing part is that the framework has many primitives, but the snapshot task is
not yet composing them in a way that matches same-slot persistent reuse.

Two runner facts are the root of the issue:

1. Non-ephemeral tasks get a per-run runtime scope override under:
   - `CI_ARTIFACTS_DIR/.runtime`
   - see `nixfied/runner/orchestrator-runtime.nix`

2. The env sandbox rewrites runtime-owned paths for each task run:
   - `TMPDIR`
   - `HOME`
   - `XDG_*`
   - `REGISTRY_ROOT`
   - `CI_ARTIFACTS_DIR`
   - `NIXFIED_SERVICE_ROOT`
   - `NIXFIED_RUNTIME_SERVICE_ROOT`
   - see `nixfied/runner/env-sandbox.nix`

So if the snapshot task uses sandbox `TMPDIR` or sandbox service-root state as the
source of truth for reusable services, then `SERVICE_REUSE_POLICY=same-slot` will not
be stable across runs. The runtime root is per-run by design.

Also note the nuance:

- stable slot/env runtime roots already exist when no per-run override is applied
- the real gap is that persistent reusable service state is not separated from the
  per-run runtime scope once the orchestrator installs `.runtime`

That is why these project-layer changes were legitimate stopgaps:

- moving `services_root_base` from `''${TMPDIR:-/tmp}/...` to `/tmp/...`
- moving Postgres socket directories to a separate short `/tmp/...` root
- treating an existing real Helios listener as reusable and letting the sync gate
  decide readiness
- failing early if the managed Helios pid dies during sync wait
- starting persisted Helios in a separate session via `setsid` / `nohup`

It was compensating for a real framework composition gap, not just papering over a
local bug.

## What Is Duplicated Today

The snapshot task currently duplicates or partially duplicates framework behavior:

- reuse-policy validation
- stable/persistent service-root selection
- Postgres startup logic
- Postgres stale pid cleanup
- Postgres short socket-dir handling
- Helios startup logic
- Helios pid-file handling
- detached child-process launching via `setsid` / `nohup`
- Helios readiness/sync gating

Some of that duplication is complete duplication, and some is "close but not quite":

- framework lifecycle gaps are uneven:
  - Postgres `start` already daemonizes and returns after readiness
  - Helios managed lifecycle is still foreground-oriented from the caller's point of view
- the snapshot task wants "ensure service is running and reusable after the task exits"
- those are related, but they are not yet the same primitive
- Helios `ready` already covers `eth_blockNumber`, but the snapshot wrapper still adds
  a stricter mainnet sync policy on top

## Core Architectural Conclusion

Do not delete `nixfied/project/module.nix`.

Instead:

- keep it as the project declaration layer
- move reusable lifecycle/reuse behavior into framework primitives
- make `mfm::portfolio::snapshot` a thin composition wrapper
- evaluate existing framework primitives (`start_service`, supervisor) before adding a
  completely new persistence path

The target split should be:

- framework:
  - stable service-root policy
  - service ensure/start-or-reuse behavior
  - persistence/detach semantics
  - health/readiness orchestration
- project wrapper:
  - MFM-specific argument contract
  - mainnet-only policy
  - MFM env wiring
  - optionally, one documented stricter Helios readiness policy if framework
    `ready` does not absorb it
  - final `mfm_cli` invocation

## Proposed Commit Sequence

Use commits, not PR-sized buckets.

### Commit 1: framework service-root policy

Goal:

- add a framework primitive that resolves persistent reusable service state
  separately from the per-run runtime scope used for `TMPDIR`, artifacts,
  and sandbox-owned service roots
- preserve the existing stable slot/env fallback roots when no per-run override
  is active

Expected scope:

- framework runner/runtime plumbing
- possibly service-policy/runtime helper code

Key constraint:

- this must preserve current per-run artifacts/tmp behavior while separating the
  location used for reusable service state

### Commit 2: framework ensure/start-or-reuse primitive

Goal:

- add a framework-level operation for "ensure this service is running under the
  requested reuse policy"

Expected behavior:

- validate reuse/owner/discovery policy
- resolve stable service root
- reuse a healthy existing instance when appropriate
- start and detach when persistence is requested
- fail only after health/ready checks fail

Candidate surfaces:

- extend generated service APIs with an `ensure` op
- adapt existing supervisor machinery for persistent service ownership
- `svc::postgres::ensure`
- `svc::helios::ensure`
- or a generic internal helper used by both

Important constraint:

- do not assume existing `start` / `full-start` are sufficient as-is
- they are not currently reuse-policy-aware
- Helios in particular still needs persistence semantics that survive task exit

This commit is the key abstraction that removes the need for the snapshot wrapper to
manually own pid files and child process sessions.

### Commit 3: refactor `mfm::portfolio::snapshot` to use framework primitives

Goal:

- replace manual service orchestration in `nixfied/project/module.nix` with
  framework composition

The wrapper should keep only:

- arg validation
- `HELIOS_NETWORK=mainnet` enforcement
- env exports needed by MFM
- optionally, one documented stricter Helios readiness check if the framework
  does not absorb that contract
- `mfm_cli --output-format json portfolio snapshot`

Everything else should move behind framework service ops/helpers.

### Commit 4: remove local duplicates and tighten tests/docs

Goal:

- delete dead manual lifecycle code from the snapshot task
- keep docs accurate
- verify same-slot behavior explicitly

Likely docs to touch:

- `docs/helios.md`
- possibly `nixfied/README.md` if new framework surfaces are user-facing

### Suggested Commit Messages

Use these as starting points:

- `nixfied: add stable service roots for same-slot reuse`
- `nixfied: add ensure/start-or-reuse service primitive`
- `mfm: refactor portfolio snapshot to compose nixfied service lifecycle`
- `mfm: remove duplicated snapshot service orchestration`

## Definition Of Done

The refactor is complete when all of the following are true:

- `mfm::portfolio::snapshot` no longer manually manages service pid files
- `mfm::portfolio::snapshot` no longer manually chooses `/tmp/...` service roots
- service reuse semantics are implemented in framework code, not project shell
- same-slot reuse still works across repeated runs
- foreground task exit does not kill intentionally reused services
- readiness ownership is explicit and consistent:
  - either framework `ready` fully absorbs the mainnet usability requirement
  - or the wrapper keeps exactly one documented stricter Helios policy

## Validation Checklist

Manual validation to run after the refactor:

```bash
NIX_ENV=0 SERVICE_REUSE_POLICY=never nix run .#mfm::portfolio::snapshot -- 0x53078a4d5618f98123CFcFCdF5c20cb2Cdf97312
NIX_ENV=0 SERVICE_REUSE_POLICY=same-slot nix run .#mfm::portfolio::snapshot -- 0x53078a4d5618f98123CFcFCdF5c20cb2Cdf97312
NIX_ENV=0 SERVICE_REUSE_POLICY=same-slot nix run .#mfm::portfolio::snapshot -- 0x53078a4d5618f98123CFcFCdF5c20cb2Cdf97312
nix run .#ready -- --service postgres --source local
nix run .#ready -- --service helios --source local
nix run .#health -- --service postgres --source local
nix run .#health -- --service helios --source local
```

Behavior to confirm:

- second `same-slot` run reuses services instead of replacing them
- stale pid files are cleaned correctly
- task exit does not tear down reused services
- JSON stdout contract stays unchanged

## Risks / Open Questions

1. Service lifecycle scripts today are foreground-oriented.
   More precisely:
   - Postgres already daemonizes cleanly on `start`
   - Helios managed lifecycle does not yet provide the persistence semantics the
     snapshot wrapper needs

   The framework may need a distinct "ensure" or "persistent start" concept rather than
   reusing the existing `start` op as-is.

2. Stable service roots should not silently conflate:
   - per-run artifacts
   - per-slot runtime scratch
   - persistent reusable service state

3. Helios has two readiness layers in practice:
   - framework health/ready
   - snapshot-specific "mainnet sync is actually usable" expectations

   Decide whether that extra sync gate belongs:
   - in framework Helios readiness
   - in a stricter framework Helios profile
   - or in the MFM wrapper as the one remaining Helios-specific policy

4. CI service startup may also benefit from the same abstraction once it exists.
   Do not expand scope immediately, but keep that follow-up in mind.

5. Existing framework primitives may already cover part of Commit 2:
   - `start_service` already knows about keep-running vs cleanup policy
   - supervisor already has a detached daemon model

   Decide whether to extend one of those paths or introduce a new shared primitive.

## Recommendation For The Next Person

- Start with framework service-root semantics first.
- Do not begin by editing the snapshot wrapper again.
- Once stable reusable roots exist in the framework, add the ensure/start-or-reuse
  primitive.
- Only then shrink `mfm::portfolio::snapshot`.

## Next Time

- Upstream "persistent reusable service root" as an explicit framework concept.
- Upstream "ensure service running under reuse policy" as a first-class service op.
- Keep project modules thin by default; avoid letting app wrappers become supervisors.
