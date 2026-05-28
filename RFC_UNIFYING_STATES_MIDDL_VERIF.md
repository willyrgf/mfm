# RFC: Unifying Framework States, Runtime Middleware, And Verification

Date: 2026-05-28

Status: proposed architecture; documentation only.

Primary input:

- `PROBLEM_MIDDLEWARE_VERIFICATION.md`

Related context:

- `RFC_STATE_OPS_PROBLEM.md`
- `RFC_TYPED_CORE_PROPOSAL_1.md`
- `PROBLEM_IMPLEMENTATION_TYPED_CORE.md`
- `DECISION_TYPED_CORE_CORRECTIVE_ARCHITECTURE.md`
- `docs/design.md`
- `docs/architecture.md`

## Executive Recommendation

Make sealed framework states the only lifecycle model, and make runtime middleware the only
run-mutating authority.

The corrected architecture is:

```text
untrusted bytes
  -> mfm-certify verifies and returns CertifiedTypedSpec
  -> mfm-runtime constructs CertifiedRuntimeSpec
  -> sealed launch/read/replay authority is minted
  -> runtime mutation middleware prepares a commit
  -> store atomically admits artifact evidence and appends typed events
```

The existing branch already has the right precedent: `NodeSpec` can carry sealed framework metadata
through `FrameworkNodeSpec`, and public-output rendering is already lowered into a certified
framework node. This RFC generalizes that model to lifecycle work:

```text
BootstrapRun
PublicOutputRender
ProjectRetentionManifest
CompleteRun
```

The runtime should not grow a parallel public "runtime transition" API for lifecycle events.
Instead, lifecycle transitions are sealed framework-state outputs interpreted by mandatory runtime
middleware.

The store remains the durable authority boundary. Artifact bytes may be staged before commit, but
artifact evidence becomes run authority only in the same store commit that references it from typed
events.

## Problem Summary

`PROBLEM_MIDDLEWARE_VERIFICATION.md` shows that the current typed branch fixed the most severe
certification issue: `CertifiedRuntimeSpec` now requires `CertifiedTypedSpec`, and app start paths
verify certified bundles through `mfm-certify`.

The remaining problem is ownership fragmentation:

- app helpers persist mandatory runtime artifacts;
- transport runners persist state output, fact, side-effect, receipt, and confirmation artifacts;
- scheduler methods append `RunStarted`, `RunCompleted`, and retention events outside node
  execution;
- public-output receipt bytes are repaired later by app code;
- replay/read authority checks are correct in spirit but owned by app helpers and public
  constructors;
- typed-store artifact evidence is recorded separately before commits in production paths.

The current shape is:

```text
state execution returns payload evidence
transport/app code persists bytes by convention
runtime validates runner output and appends typed events
store validates event/evidence preconditions
```

The target shape is:

```text
state execution returns staged bytes, sealed handles, and payload intent
runtime middleware verifies, persists/promotes staged bytes, and derives scheduler-owned payloads
store atomically admits evidence and appends the referencing event batch
```

## Core Principle

There is only one production path for run mutation:

```text
CertifiedRuntimeSpec
  + sealed execution context
  + staged artifact bytes/handles
  + validated payload intent
  -> runtime mutation middleware
  -> PreparedTypedCommit
  -> store committed batch
```

User/domain runners, app services, CLI, REST, tests, and conformance helpers must not mint durable
run authority directly.

This does not mean fewer checks. It means fewer owners for the checks. The certifier, runtime,
artifact store, typed store, and replay/read boundaries continue to defend hostile persisted data.
App and transport code stop being authority surfaces.

## Runtime Mutation Middleware

Introduce a single runtime mutation middleware layer below scheduling.

It has two entry contexts:

```text
GenesisContext
  CertifiedRuntimeSpec
  RunId
  prepared launch artifacts and seed/config materialization
  RequiredRunState::Absent

StartedRunContext
  CertifiedRuntimeSpec
  VerifiedRunStream
  ProjectionSnapshot
  runnable certified node
  RequiredRunState::NotCompleted
```

Both contexts flow through the same commit preparation machinery:

```text
sealed framework/user execution intent
  -> staged artifacts or sealed staged handles
  -> payload derivation
  -> runner/framework output validation
  -> artifact byte verification and CAS persistence/promotion
  -> PreparedTypedCommit
  -> atomic store commit with artifact evidence and typed event payloads
```

`BootstrapRun` still uses a genesis context because there is no existing run stream. It should not
use a second persistence implementation. Its only specialness is the precondition and first-batch
ordering:

```text
RequiredRunState::Absent
RunStarted is ordinal 0
```

Normal node scheduling requires an existing `RunStarted`; runtime mutation middleware does not.

## Sealed Framework States

Extend `FrameworkNodeSpec` so lifecycle work is represented in the certified spec:

```text
FrameworkNodeSpec::BootstrapRun
FrameworkNodeSpec::Bridge
FrameworkNodeSpec::PublicOutputRender
FrameworkNodeSpec::ProjectRetentionManifest
FrameworkNodeSpec::CompleteRun
```

Only framework lowering/certification can create these variants. Runtime resolves them to built-in
framework runners or directives. User/domain runners remain unable to emit scheduler-owned payloads
such as `RunStarted`, `RunCompleted`, `RetentionManifestProjected`, `RetentionRefsAppended`, or
`StateAttemptStarted`.

### BootstrapRun

`BootstrapRun` is part of the certified spec, but it is executed through a genesis context.

The first committed batch is:

```text
RunStarted
StateAttemptStarted(BootstrapRun)
CellProduced(bootstrap receipt)
StateAttemptCompleted(BootstrapRun)
RetentionRefsAppended(reason = RunStarted)
```

`RunStarted` remains first because it is the root evidence for resume, replay, public-output reads,
and retention.

`BootstrapRun` is hash-defining, but it must not contain post-hash launch evidence:

- no run id;
- no spec artifact id;
- no certificate artifact id or certificate digest;
- no runner or adapter executable identities;
- no concrete seed artifact evidence;
- no same-batch event ids or commit fingerprints.

The dependency remains one-way:

```text
TypedExecutionSpec including BootstrapRun
  -> spec hash
  -> spec/certificate artifacts
  -> launch evidence
  -> RunStarted
```

This avoids self-addressing cycles where the spec hash would depend on artifact identities that are
derived from the same spec bytes.

The bootstrap receipt should bind launch evidence compactly, but only evidence available after the
spec hash exists. Its exact schema is an implementation detail.

### PublicOutputRender

`PublicOutputRender` is already the correct precedent: certification lowers it as a framework node,
and runtime executes it through a built-in runner.

The remaining leak is byte persistence. The framework renderer must return staged receipt bytes or
a sealed staged artifact handle. Runtime middleware then:

- rebuilds or verifies the deterministic public-output receipt bytes;
- verifies evidence against the output cell and `PublicOutputProduced` payload;
- writes/promotes bytes to the artifact store;
- admits evidence in the same commit as `CellProduced`, `PublicOutputProduced`, and
  `StateAttemptCompleted`.

`build_public_output_receipt_artifact` remains useful as deterministic validation logic. It should
not be a normal app-side post-commit repair path.

### ProjectRetentionManifest

`ProjectRetentionManifest` is a sealed framework node.

Its normal `NodeSpec` input should express readiness, such as dependency on the public-output
receipt cell. Its actual data source is sealed runtime projection authority:

```text
authoritative run stream
  -> rebuilt ProjectionSnapshot
  -> retention manifest bytes/evidence
```

Do not expose the run stream or projection as public user-state input.

The terminal commit shape is:

```text
StateAttemptStarted(ProjectRetentionManifest)
CellProduced(retention projection receipt)
StateAttemptCompleted(ProjectRetentionManifest)
RetentionManifestProjected
RetentionRefsAppended(reason = ManifestProjection)
```

The retention manifest must hash the pre-projection stream. It must not include its own
`RetentionManifestProjected` event, same-batch receipt event ids, or commit fingerprint.

### CompleteRun

`CompleteRun` is a sealed framework node at the lifecycle tail.

It produces a completion receipt cell, and runtime middleware derives the existing `RunCompleted`
payload in the same terminal commit:

```text
StateAttemptStarted(CompleteRun)
CellProduced(completion receipt)
StateAttemptCompleted(CompleteRun)
RunCompleted
```

`RunCompleted` remains a kernel event because store projection uses it to mark terminal run state.
It is no longer produced by a scheduler-only `complete_run` method.

The lifecycle tail should be certified as:

```text
PublicOutputRender -> ProjectRetentionManifest -> CompleteRun
```

This removes app-owned scheduling order and makes public output, retention, and completion a single
certified framework program tail.

## Artifact Staging And Byte Authority

Artifact stores are verified content-addressed byte stores. They are not semantic run authority by
themselves.

The invariant is:

```text
artifact bytes may be staged before commit
artifact evidence becomes run authority only when committed by the typed store
```

Therefore:

- transport runners must not receive `put_verified_artifact` or equivalent write-capable artifact
  store traits;
- app services must not pass filesystem artifact stores through production runner sink adapters;
- runners return staged artifacts, staged descriptors, or sealed staged handles;
- runtime middleware verifies staged bytes against evidence and event payloads before commit;
- the store atomically admits evidence and appends the event batch referencing it;
- orphan artifact-store bytes are allowed;
- orphan run-store artifact evidence is not allowed in production paths.

For small artifacts, `ErasedRunnerOutput` can carry staged bytes directly:

```text
StagedArtifact {
  bytes,
  descriptor or evidence intent,
  binding kind
}
```

For large artifacts, runners use sealed per-attempt handles:

```text
StagedArtifactHandle {
  node_id,
  attempt_id,
  artifact_role,
  schema/semantic producer constraints,
  digest/byte_len after finalization
}
```

The handle must not expose general artifact-store writes. It is a staging capability bound to one
node attempt and one artifact role.

Managed platform writes to MFM artifact storage are distinct from external side effects. Persisting
fact, output, intent, submission, receipt, confirmation, and diagnostic bytes is platform
persistence and belongs under runtime middleware even when the bytes describe external systems.

## Store Commit Contract

Production store append should admit artifact evidence and append the referencing typed event batch
atomically.

The current conceptual split:

```text
record_artifact_evidence
append_typed_run_commit
```

should become a production API shaped like:

```text
append_prepared_typed_commit(PreparedTypedCommit)
```

or:

```text
append_typed_run_commit(TypedCommitRequest {
  admitted_artifact_evidence,
  payloads,
  preconditions,
})
```

The exact Rust API can vary, but the authority rule cannot: no production path should record
run-store artifact evidence without the commit that references it.

Separate `record_artifact_evidence` remains useful for:

- tests that deliberately build corrupt streams;
- migrations;
- repair tooling;
- low-level store contract fixtures.

It should not remain part of positive app, CLI, REST, transport, or conformance production paths.

The store still owns:

- sequence and ordinal assignment;
- event ids;
- logical keys;
- commit-key idempotency;
- payload canonical hashes;
- required run/cell/side-effect/public-output preconditions;
- event/projection consistency;
- atomic projection updates.

## Verification Ownership

Permanent persisted-data defenses stay. Temporary owner leaks move behind sealed authority types.

### Certifier

`mfm-certify` owns serialized spec/certificate verification. Persisted spec bytes and certificate
bytes are hostile until verification returns `CertifiedTypedSpec`.

This remains true at:

- start;
- resume;
- replay;
- public-output reads;
- retention projection.

### Runtime

Runtime owns executable authority derived from `CertifiedTypedSpec`.

New private-field authority types should encode preconditions:

```text
PreparedRunLaunch
  minted from CertifiedTypedSpec plus launch artifact materialization

VerifiedRunStream
  minted from CertifiedRuntimeSpec plus authoritative store stream validation

PreparedTypedCommit
  minted only by runtime mutation middleware
```

`RunStartEvidence` and `TypedRunStartRequest` should either become opaque or be replaced by
`PreparedRunLaunch`. Production app code should not assemble launch evidence directly.

### Replay

Replay authority must require certifier-backed spec authority, not a hash-only envelope plus caller
convention.

Use a sealed authority type:

```text
ReplayReadAuthority
  CertifiedTypedSpec
  VerifiedRunStream
  retained artifact evidence from committed stream/projection
```

`ReplayAuthority::new` and `ReplayBroker::from_run_stream` should not remain public production
constructors accepting arbitrary hash-only spec data.

### Public Output Reads

`PublicOutputReadAuthority` should be minted from:

```text
CertifiedRuntimeSpec
VerifiedRunStream
rebuilt public-output projection
artifact evidence checks
```

Rendered JSON remains a cache/output surface. Typed terminal cells plus public-output events remain
authority.

### App, CLI, And REST

App, CLI, and REST may:

- parse request bytes;
- select stores, artifact stores, registries, runners, and capabilities;
- call certifier/runtime/replay authority constructors;
- shape DTOs and public responses.

They must not:

- persist mandatory runtime artifacts as authority;
- schedule lifecycle projection work directly;
- pass write-capable artifact-store sinks to runners;
- construct replay/read/runtime authority from raw stream inspection;
- repair public-output or retention artifacts after the fact as a normal path.

## Typed Input And Config Materialization

Config and ordinary input artifacts should be materialized before domain runner invocation through
a framework-owned typed runner adapter.

Runtime core can remain domain-free by owning the generic evidence checks and invoking typed
transport/domain adapters with already verified materialized inputs.

The materializer verifies:

- config artifact id, digest, byte length, media type, and schema id against `ConfigRef`;
- input cell terminal evidence against certified binding trees;
- schema id, semantic type id, artifact role, producer node/seed, digest, and byte length;
- no uncommitted artifact evidence is used as authority.

Domain runners keep:

- hostile external data validation;
- domain config semantic validation;
- side-effect protocol checks;
- replay-specific external evidence validation.

## What Remains Special

The architecture intentionally preserves a small set of special boundaries:

- certified bundle verification at serialized boundaries;
- genesis context for the first batch;
- built-in framework runner/directive resolution;
- artifact-store byte/evidence integrity;
- typed-store atomic commit/projection validation;
- read/resume/replay authority reconstruction from hostile persisted history.

The bootstrap first batch is special because there is no prior `RunStarted` stream authority. It is
not special because it bypasses middleware. It uses the same runtime mutation middleware through a
`GenesisContext`.

## Runtime Path Cleanup

The implementation should delete or demote every alternative production path that can mutate a run
outside the runtime mutation middleware.

The cleanup rule is:

```text
if a path can append run events, admit run-store artifact evidence, persist mandatory runtime
artifacts, or derive lifecycle authority, it must go through runtime mutation middleware
```

The following should not remain as production APIs:

- standalone scheduler `start_run` logic that builds `RunStarted` from caller-assembled evidence;
- standalone scheduler `complete_run` logic that appends `RunCompleted` without a `CompleteRun`
  framework state;
- standalone retention projection append logic that is callable without
  `ProjectRetentionManifest`;
- app-side public-output receipt persistence or retention-manifest persistence;
- transport runner artifact sinks that expose `put_verified_artifact`;
- production store paths that call `record_artifact_evidence` before a later event append;
- positive conformance helpers that manually persist spec/config/public-output artifacts or append
  lifecycle events outside the same app-agnostic runtime/framework primitives used by production.

Lower-level APIs may remain only when their names, visibility, and tests make the bypass explicit:

- `#[cfg(test)]` fixtures;
- corruption and tamper tests;
- migration or repair tooling;
- storage contract tests that intentionally validate invalid or partial states.

No positive app, CLI, REST, transport, or conformance path should teach a second runtime model.

## Required Architectural Tests

The implementation must add an integration or conformance test that proves execution and
persistence both flow through the unified model.

The test should execute a certified spec that contains at least:

- `BootstrapRun`;
- one ordinary user/domain state;
- `PublicOutputRender`;
- `ProjectRetentionManifest`;
- `CompleteRun`.

The test should use production-path app/runtime APIs with an instrumented store and artifact
stager. It should assert:

- every executable unit in the certified spec is represented by a state attempt in the run stream;
- `RunStarted` appears only in the genesis batch and that same batch contains the `BootstrapRun`
  attempt lifecycle;
- scheduler-owned lifecycle payloads are never committed as standalone scheduler transitions;
- `RunCompleted` appears only in the same commit as `CompleteRun` terminal evidence;
- `RetentionManifestProjected` appears only in the same commit as `ProjectRetentionManifest`
  terminal evidence;
- every event-referenced artifact was supplied to middleware as staged bytes or a sealed staged
  handle;
- every event-referenced artifact evidence row was admitted in the same store commit that first
  referenced it;
- no positive-path call records artifact evidence before a later event append;
- direct runner/app artifact-store writes are impossible in production types.

This test is the behavioral backstop for the central claim of this RFC:

```text
all execution is state execution
all state persistence is middleware persistence
```

It should be paired with compile-time/API tests where possible:

- downstream code cannot construct `PreparedTypedCommit`;
- downstream code cannot construct `PreparedRunLaunch` from raw evidence;
- production runners cannot name or receive a write-capable artifact-store trait;
- replay authority cannot be constructed from a hash-only spec envelope.

## Rejected Alternatives

### Make BootstrapRun A Completely Ordinary Scheduled Node

This creates a cycle:

```text
ordinary node attempt requires RunStarted
BootstrapRun is supposed to produce RunStarted
```

It also risks a self-addressing hash cycle if the spec includes artifact identities derived from
the spec bytes.

### Add Events Before RunStarted

This weakens the root invariant. Resume, replay, public-output reads, and retention would need to
interpret pre-run attempts before the certified run authority is bound.

### Add A Separate Genesis Stream

This moves the special case to another stream and creates a second authority surface. It does not
reduce conceptual complexity.

### Include Launch Evidence In The Certified Spec

This makes certification per-run and environment-specific, prevents reusable certified specs, and
does not solve self-addressing artifact identity cycles.

### Use Placeholder Or Two-Phase Hashes

This adds arbitrary hash semantics and undermines simple content addressing.

### Keep Direct Artifact Sinks But Audit Them

This preserves the current authority leak under another name. A write-capable runner sink can
persist bytes outside the runtime commit middleware.

## Implementation Sequence

Prefer small, reviewable commits:

1. Define the runtime mutation middleware boundary and staged artifact model.
2. Add atomic commit-with-evidence support to in-memory and durable typed stores.
3. Convert public-output receipt persistence to staged framework-runner output.
4. Replace production transport artifact write sinks with staged artifacts/handles.
5. Add `ProjectRetentionManifest` and move retention projection into a sealed framework state.
6. Add `CompleteRun` and derive `RunCompleted` from its sealed framework output.
7. Add `BootstrapRun`, `GenesisContext`, and `PreparedRunLaunch`.
8. Move launch/read/replay authority checks behind sealed runtime/replay constructors.
9. Update app, CLI, REST, conformance, and positive integration paths to use the unified
   authority APIs.
10. Confine direct store mutation and separate evidence recording to explicit test/migration
    fixtures.
11. Add the architectural coverage test proving that every executed state used runtime mutation
    middleware and that every referenced artifact was admitted with the same commit.

## Acceptance Criteria

The architecture is corrected when:

- all lifecycle work is represented as sealed framework nodes or the sealed bootstrap genesis
  batch;
- `RunStarted` is committed only through `PreparedRunLaunch` and genesis middleware;
- public-output receipt bytes are persisted by runtime middleware, not app repair code;
- retention projection is a sealed framework node using sealed runtime projection input;
- `RunCompleted` is derived from `CompleteRun`, not scheduler-only completion;
- production runners have no write-capable artifact-store traits;
- `ErasedRunnerOutput` or its replacement carries staged artifacts or sealed handles, not only
  evidence;
- production store append admits artifact evidence and event payloads atomically;
- orphan artifact bytes are never treated as run authority;
- `record_artifact_evidence` and direct `append_typed_run_commit` are absent from positive
  app/transport/CLI/REST/conformance paths;
- replay authority requires certifier-backed authority plus committed retained evidence;
- public-output read authority is minted only from verified certified authority and validated run
  stream evidence;
- raw status/stream DTOs cannot be passed where runtime, replay, public-output, or retention
  authority is required;
- an architectural coverage test proves that every executable node in a representative certified
  run executed as a state attempt and used runtime mutation middleware for persistence;
- all former scheduler/app/transport alternative runtime mutation paths are deleted, private, or
  confined to explicit test/migration/corruption fixtures;
- tests that directly mutate stores are clearly named as negative, corruption, migration, or
  low-level store contract fixtures.

## Residual Design Details

These remain implementation details and should not reopen the architecture:

- exact bootstrap receipt schema;
- exact `PreparedTypedCommit` Rust API shape;
- exact staged-artifact inline vs streaming handle API;
- exact `ProjectRetentionManifestNodeSpec` fields;
- exact crate placement for typed config/input materialization;
- retention and confidentiality rules for prepared invocation artifacts that may contain sensitive
  operational material.

## Final Definition

MFM run mutation has one model:

```text
certified user state or sealed framework state
  -> runtime mutation middleware
  -> staged artifacts verified against typed payloads
  -> atomic store commit
  -> append-only run stream authority
```

Bootstrap uses a genesis context, not a bypass. Public output, retention, completion, replay, and
read authority all flow from the same certified state and store-owned event model.
