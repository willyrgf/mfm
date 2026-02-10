# NIX_STATE_PROPOSAL.md

## Goal

Enable an MFM state to execute an external implementation in a reproducible environment, producing a JSON result that can be written into the MFM context and passed to subsequent states, while preserving the `REDESIGN.md` Milestone 1 invariants:

- No ambient IO (states must not do direct network/FS/process IO).
- Live mode records deterministic facts as artifacts; Replay mode reuses recorded facts.
- Canonical JSON (no floats) for hashed/persisted structured data.
- No secrets persisted (manifests, events, artifacts, snapshots, error details).
- Resume correctness via kernel attempt envelopes.

This supports use cases like generating configuration, producing a transaction signature, or performing a deterministic computation using tooling not implemented in Rust.

## Non-Goal (v1)

- v1 does not implement a full `nix run` / `nix flake check` workflow from inside the runtime.
- v1 does not depend on invoking the `nix` CLI.
- v1 does not attempt to use the `nix-rust/nix` crate as a Nix package-manager interface (it is POSIX/syscall bindings).

## Key Design Choice

Because we want reproducibility but must avoid invoking the `nix` CLI, v1 executes a pinned Nix store program path, typically:

- `program_path = "/nix/store/<hash>-pkg/bin/<app>"`

This is reproducible because the Nix store path is content-addressed by Nix, and MFM will record the execution result as a fact payload artifact.

Future milestones can add a "realize if missing" backend via Nix daemon protocol/bindings (still without CLI).

## Where It Fits (REDESIGN.md-aligned)

We do not change the runtime/planner core types. We add:

1. A generic `State` implementation: `NixAppState` (external program state).
2. A dedicated operation: `nix_app` which expands deterministically to a `StateGraph` containing `NixAppState`.
3. A `LiveIoTransportFactory` backend that supports an IO namespace (e.g. `"exec"`), so:
   - states use `io.call(...)`
   - live execution is recorded as facts
   - replay returns recorded facts

This leverages existing `mfm-machine` concepts:

- `IoCall` / `IoProvider` for live/replay
- `FactKey` for recording/replay
- context snapshots via staged writes (`StagedContext`)
- secrets scanning + redaction rules

## External App Protocol (stdin JSON -> stdout JSON)

### Contract (external implementation)

- Reads exactly one JSON object from `stdin`.
- Writes exactly one JSON value to `stdout` (typically an object).
- Writes logs to `stderr` (not persisted by MFM).
- Exit code:
  - `0`: success
  - non-`0`: failure (mapped to `StateError`)

### Request schema (example)

```json
{
  "mfm": {
    "run_id": "...",
    "state_id": "machine.step.run",
    "op_path": "machine.step",
    "attempt": 0,
    "io_mode": "live"
  },
  "input": { "...": "..." }
}
```

### Response schema (example)

```json
{
  "ok": true,
  "result": { "...": "..." },
  "meta": { "warnings": [] }
}
```

### Persisted-surface constraints

- The response must be valid JSON.
- If recorded as a fact payload artifact and/or written into context snapshots:
  - must be canonical-JSON hashable (no floats)
  - must not contain secrets (enforced by existing secret scanning)

## IO Integration (facts + replay)

### Namespace

Use a single namespace, e.g. `namespace = "exec"`.

### Exec request payload (typed)

`IoCall.request` encodes:

```json
{
  "kind": "run_program_v1",
  "program_path": "/nix/store/.../bin/app",
  "argv": ["--flag", "x"],
  "stdin_json": { "...": "..." },
  "timeout_ms": 300000,
  "env": { "SAFE_VAR": "value" }
}
```

### Fact key stability

`NixAppState` MUST provide an explicit `fact_key` so:

- Live mode records the response deterministically.
- Replay mode can return the recorded result without re-running.

Recommended derivation:

- Canonicalize the exec request (excluding attempt).
- Hash it (e.g. SHA-256 hex).
- Build key:

`mfm:exec|state:<state_id>|req:<sha256(canonical_json(request))>`

### Live mode behavior

The Live IO transport:

- executes the program at `program_path`
- passes `stdin_json` on stdin
- parses stdout JSON into `IoResult.response`
- LiveIo records the response as a fact payload artifact keyed by `fact_key`

### Replay mode behavior

Replay IO:

- ignores exec details
- returns the recorded payload for the `fact_key`
- errors with `MissingFact` if absent (retryable per run config)

## State Behavior (writing outputs into context)

`NixAppState` does:

1. Read selected input keys from context (config controlled).
2. Build request JSON and call:
   - `io.call(IoCall { namespace: "exec", request: ..., fact_key: Some(...) })`
3. Write outputs to context under namespaced keys:
   - default: `ContextKey("<op_path>.result") = response.result`
   - optional: `ContextKey("<op_path>.exec_fact_payload_id") = recorded_payload_id`
   - optional: `ContextKey("<op_path>.meta") = response.meta`

Large payloads should remain as artifacts referenced by id, not duplicated into context.

State meta defaults:

- tag: `EXECUTE`
- side effects: `ApplySideEffect` (configurable)
- idempotency: `None` (configurable)

## Operation: `nix_app` (configuration-driven)

Implement an op with:

- `op_id = "nix_app"`
- `op_version = "v1"`

`op_config` schema:

```json
{
  "program_path": "/nix/store/.../bin/app",
  "argv": [],
  "stdin_from_context": ["machine.step1.some_key"],
  "write_result_to": "result",
  "timeout_ms": 300000,
  "side_effects": "apply|readonly|pure",
  "idempotency_key": "optional-stable-string"
}
```

`expand()` is deterministic and produces a single state:

- `state_local_id = "run"`
- `state_id = "<op_path>.run"`

## Prevalidations (no CLI)

Before execution, validate:

- `program_path` exists
- `program_path` is executable
- `program_path` starts with `/nix/store/` (default allowlist; can be extended)
- timeout is within policy bounds

Optional later:

- allowlist of env vars using `RunManifest.build.env_allowlist`

## Tests

### Fast lane (no Nix required)

- Fake `LiveIoTransport` that returns deterministic JSON.
- Assertions:
  - context receives expected keys/values
  - `fact_key` stability dedupes across retries (no double execution)

### Parity lane (opt-in, Nix environment)

- Package a tiny external app (echoes input to output).
- Validate end-to-end:
  - live run records the output as a fact payload
  - replay returns the same output without executing

## Assumptions / Defaults (v1)

- No `nix` CLI invocation.
- External execution is via pinned `/nix/store/...` program paths.
- External outputs must be non-secret and canonical-JSON compatible if persisted.
- "Only apps in this repo flake" is interpreted as: repo tooling produces the store path; MFM runs that store path.

## Follow-ups

1. Add a backend to realize a store path via Nix daemon protocol/bindings (no CLI).
2. Add a v2 config mode that accepts flake app refs (likely requires CLI or deep bindings).
3. Populate `RunManifest.build.{flake_lock_hash,cargo_lock_hash,rustc_version,target_triple}` automatically in the launcher for stronger provenance.

## References

- `REDESIGN.md` (v4): execution/runtime contracts and Milestone 1 rules.
- https://github.com/nix-rust/nix (Unix syscall bindings crate, not a Nix package-manager API)

