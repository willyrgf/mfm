# NIX_STATE_PROPOSAL.md

## Goal

Enable an MFM state to execute an external implementation in a reproducible environment,
producing a JSON result that can be written into MFM context and consumed by later states,
while preserving `REDESIGN.md` Milestone 1 invariants:

- No ambient IO in state logic.
- Live mode records deterministic facts; Replay mode reuses recorded facts.
- Canonical JSON for persisted structured data (no floats).
- No secrets in persisted surfaces.
- Resume correctness through attempt envelopes and fact reuse.

This version adds a **real preflight path for flake apps** such as:

- `github:willyrgf/mfm#jq_fmt_example`

so users can understand that the app may be hosted outside the current working repository.

## What Is Implemented

The codebase now supports a two-step model for `nix_app`:

1. Resolve and validate a Nix app ref (optional) via `nix.exec`.
2. Execute the resolved pinned program path via `exec`.

Relevant implementation points:

- `crates/ops/nix-app-op/src/lib.rs`
- `crates/machine/src/nix_exec_transport.rs`
- `crates/machine/src/exec_transport.rs`

Runtime wiring (CLI + REST API) routes both namespace groups:

- `nix` -> `NixFlakeTransportFactory`
- `exec` -> `ExecProgramTransportFactory`

## Design Overview

### `nix_app` state input modes

`nix_app` accepts exactly one source of executable:

1. `program_path` mode (direct store path)
2. `app` mode (flake app reference)

Validation rules:

- exactly one of `program_path` or `app`
- `program_path` must start with `/nix/store/`
- `app` must be non-empty and contain `#`

### Two-phase execution

If configured with `app`:

1. Call `io.call(namespace="nix.exec")` with `resolve_flake_app_v1`.
2. Receive `{ "program_path": "/nix/store/.../bin/..." }`.
3. Call `io.call(namespace="exec")` with `run_program_v1`.

If configured with `program_path`, step 1 is skipped.

Both IO calls include explicit `fact_key`, so live/replay behavior is deterministic.

## IO Contracts

### Namespace: `nix.exec`

Request (`kind = resolve_flake_app_v1`):

```json
{
  "kind": "resolve_flake_app_v1",
  "app": "github:willyrgf/mfm#jq_fmt_example",
  "timeout_ms": 300000
}
```

Response:

```json
{
  "program_path": "/nix/store/<hash>-jq-<ver>/bin/jq"
}
```

Live behavior:

- policy-check app ref prefix using `manifest.run_config.nix_flake_allowlist`
  - default allowlist includes `github:willyrgf/mfm`
- parse flake ref and app fragment
- resolve app program via `nix eval --raw <flake>#apps.<system>.<name>.program`
  - if fragment is already `apps.<system>.<name>`, use it directly
  - if fragment ends with `.program`, use it directly
- verify resolved path starts with `/nix/store/`
- run `nix build --no-link <program_path>` to ensure it is realized/compiling
- return `program_path`

Replay behavior:

- return previously recorded payload for the `fact_key`
- fail with `MissingFact` if absent

### Namespace: `exec`

Request (`kind = run_program_v1`):

```json
{
  "kind": "run_program_v1",
  "program_path": "/nix/store/.../bin/app",
  "argv": ["-S", "."],
  "stdin_json": {"b": 1, "a": 2},
  "timeout_ms": 300000,
  "env": {}
}
```

Response:

- arbitrary JSON value parsed from stdout

Live behavior:

- policy-check program path prefix (`/nix/store/` by default)
- preflight executable checks using Rust `nix` crate (`access(F_OK|X_OK)`)
- execute program and parse stdout JSON

Replay behavior:

- same as any deterministic IO call keyed by fact key

## Fact Key Strategy

`nix_app` uses stable keys per request hash:

- preflight key:
  - `mfm:nix:preflight|state:<state_id>|req:<sha256(canonical_json(req))>`
- exec key:
  - `mfm:exec|state:<state_id>|req:<sha256(canonical_json(req))>`

This gives deterministic replay and dedupe semantics across retries/resume.

## `nix_app` Operation Config

```json
{
  "program_path": "/nix/store/.../bin/app",
  "app": "github:willyrgf/mfm#jq_fmt_example",
  "argv": [],
  "stdin_json": {},
  "timeout_ms": 300000,
  "write_result_to": "result"
}
```

Notes:

- `program_path` and `app` are mutually exclusive.
- `write_result_to` defaults to `result`.
- state writes stdout JSON into context at `<op_path>.<write_result_to>`.

## Run Policy: `nix_flake_allowlist`

Flake ref allowlisting is configured at run policy level and persisted in the manifest:

- `RunManifest.run_config.nix_flake_allowlist: Vec<String>`

Example:

```json
{
  "run_config": {
    "nix_flake_allowlist": [
      "github:willyrgf/mfm",
      "path:/absolute/path/to/another/flake"
    ]
  }
}
```

`nix.exec` resolves allowlist from the current run manifest before validating `app`.

## Real Example: `github:willyrgf/mfm#jq_fmt_example`

### Flake app

The repository exposes a real app:

- `jq_fmt_example` in `nixfied/local/default.nix`

Behavior:

- reads JSON from stdin
- writes formatted JSON with sorted keys via `jq -S '.'`

You can run it directly from any repository location:

```bash
printf '{"b":1,"a":2}' | nix run github:willyrgf/mfm#jq_fmt_example
```

### `nix_app` config using external flake ref

```json
{
  "app": "github:willyrgf/mfm#jq_fmt_example",
  "argv": [],
  "stdin_json": {"b": 1, "a": 2},
  "write_result_to": "result"
}
```

Execution flow:

1. `nix.exec` preflight resolves and realizes the app.
2. `exec` runs the resolved `/nix/store/.../bin/jq`.
3. Context receives `<op_path>.result` with sorted-key JSON output.

## Preflight Analysis Contract

For `app = github:willyrgf/mfm#jq_fmt_example`, preflight ensures:

1. reference shape is valid (`<flake>#<fragment>`)
2. app ref is allowlisted by policy
3. app resolves to a concrete `program` path in `/nix/store/`
4. the resulting path can be realized/compiled (`nix build --no-link`)
5. final executable existence/execute-bit is checked before spawn

This is intentionally a **real validity check**, not a string-only validation.

## Security and Invariants

- No stdout/stderr/request echo in transport error messages.
- Responses recorded as facts must pass existing canonical JSON + secret checks.
- Replay never re-runs external commands when a fact is present.
- State logic remains IO-abstracted (`io.call`) with no ambient process calls.

## Non-Goals (Current)

- No support for arbitrary unallowlisted Git refs by default.
- No broad "execute any flake app from the internet" policy by default.
- No persisted secrets in manifests/events/artifacts/snapshots.

## Follow-ups

1. Add richer preflight diagnostics artifact (sanitized) for operator debugging.
2. Add optional pinning constraints (e.g., locked rev) for stricter provenance.
3. Add dedicated parity-lane e2e that resolves `github:willyrgf/mfm#jq_fmt_example` over network.

## References

- `REDESIGN.md` (v4): execution/runtime contracts and Milestone 1 invariants.
- `crates/machine/src/nix_exec_transport.rs`
- `crates/machine/src/exec_transport.rs`
- `crates/ops/nix-app-op/src/lib.rs`
- `nixfied/local/default.nix`
