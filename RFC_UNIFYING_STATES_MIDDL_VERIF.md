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
events. This is a breaking cleanup: the production store mutation surface should converge on one
prepared-commit API, and the split evidence/append model should be removed from execution.

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

User/domain runners, app services, CLI, REST, positive tests, and conformance helpers must not mint
durable run authority directly.

No second authority rule:

- every production run mutation must be represented as sealed state execution intent;
- every production run mutation must produce `PreparedTypedCommit`;
- every production run mutation must enter durable history through
  `append_prepared_typed_commit(PreparedTypedCommit)`;
- app, CLI, REST, transports, scheduler helpers, and positive conformance cannot own alternate
  lifecycle, artifact-evidence, replay, render, retention, or completion mutation paths;
- synthetic history construction for corruption tests, migrations, or repair is not execution and
  must not share execution traits, constructors, or call paths.

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
use a second persistence implementation. The goal is to normalize bootstrap at the state level:
`BootstrapRun` is a real sealed framework state with the ordinary attempt lifecycle and a framework
receipt cell. Its pre-execution middleware is genesis-specific only because it must prepare the
first commit before a `VerifiedRunStream` can exist.

The specialness is confined to the genesis precondition, launch materialization, and first-batch
ordering:

```text
RequiredRunState::Absent
RunStarted is ordinal 0
```

Normal node scheduling requires an existing `RunStarted`; runtime mutation middleware does not.
The bootstrap path therefore has a dedicated genesis middleware entrypoint, but it must still emit
a `PreparedTypedCommit` through the same artifact staging, payload validation, and store commit
boundary used by all other state executions.

## Sealed Framework States

Extend `FrameworkNodeSpec` so lifecycle work is represented in the certified spec:

```text
FrameworkNodeSpec::BootstrapRun
FrameworkNodeSpec::Bridge
FrameworkNodeSpec::PublicOutputRender
FrameworkNodeSpec::ProjectRetentionManifest
FrameworkNodeSpec::CompleteRun
```

Only framework lowering/certification can create these variants as executable authority. Runtime
resolves them to built-in framework runners plus, for `BootstrapRun` only, the genesis
pre-execution middleware needed to create the first run stream commit. User/domain runners remain
unable to emit scheduler-owned payloads such as `RunStarted`, `RunCompleted`,
`RetentionManifestProjected`, `RetentionRefsAppended`, or `StateAttemptStarted`.

Because specs are persisted data, serialized framework variants are not authority by themselves.
Certification must validate framework descriptor identity, topology, metadata, lifecycle ordering,
and framework-only payload permissions before any lifecycle variant reaches runtime authority.
Runtime must repeat the framework-node contract checks when constructing `CertifiedRuntimeSpec`.

### BootstrapRun

`BootstrapRun` is part of the certified spec, but it is executed through a genesis context.
It should be treated as an ordinary sealed framework state wherever possible: it has a certified
node identity, framework descriptor, attempt lifecycle, output/receipt cell, artifact bindings, and
middleware-owned terminal commit. The only non-ordinary part is the pre-execution middleware that
creates the root run authority.

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
spec hash exists. Its exact schema is an implementation detail, but the authority rule is not:
production code must not construct `RunStarted` from caller-assembled evidence outside
`PreparedRunLaunch` and the bootstrap genesis middleware.

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

`CompleteRun` is the final sealed framework node at the lifecycle tail.

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

The lifecycle tail is certified as:

```text
PublicOutputRender -> ProjectRetentionManifest -> CompleteRun
```

This ordering is deliberate. `ProjectRetentionManifest` hashes the authoritative pre-completion
stream, including public-output evidence and all retention-relevant prior artifacts, but excluding
its own projection events and the later `RunCompleted` event. `CompleteRun` is then the final
normal framework state execution that marks the run terminal. No framework state executes after
`RunCompleted`, so completion does not require a post-terminal retention special case.

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

Large-handle finalization is part of runtime middleware authority:

- a handle is minted for exactly one run id, node id, attempt id, artifact role, and binding kind;
- finalization fixes digest, byte length, media/schema/semantic identities, and producer evidence;
- a finalized handle cannot be reused by another attempt, role, or commit;
- middleware verifies the finalized evidence against the certified node and typed payloads before
  preparing the commit;
- failed commits may leave artifact-store bytes behind, but must not leave admitted run-store
  evidence.

Managed platform writes to MFM artifact storage are distinct from external side effects. Persisting
fact, output, intent, submission, receipt, confirmation, and diagnostic bytes is platform
persistence and belongs under runtime middleware even when the bytes describe external systems.
For side-effect evidence, staged artifacts must also bind the side-effect ledger key, invocation
epoch, attempt id, and evidence phase so receipt or confirmation bytes cannot be admitted under a
different side-effect execution.

## Store Commit Contract

Production store mutation is one breaking API:

```text
append_prepared_typed_commit(PreparedTypedCommit)
```

The old conceptual split is removed from production:

```text
record_artifact_evidence
append_typed_run_commit
```

No production API should preserve that split. The store admits artifact evidence and appends the
referencing typed event batch atomically from `PreparedTypedCommit`, or it appends nothing.

Synthetic evidence insertion is not an execution API. If corruption testing, migration, or repair
needs to construct partial history, it must live in a separate test/tool surface that cannot be
called by runtime scheduling, app services, transport runners, positive conformance, CLI, or REST.
The only allowed non-execution surfaces are:

- `#[cfg(test)]` fixtures that deliberately build corrupt streams;
- explicitly named migration tooling;
- explicitly named repair tooling;
- low-level storage contract tests that validate invalid or partial states.

It must not share the same trait, type, constructor, or ergonomic API surface as normal execution.
Any stream produced, repaired, or corrupted by these surfaces is still hostile persisted history
until the normal certified-spec, `VerifiedRunStream`, replay, or public-output authority
constructors validate it. Tooling does not mint runtime, replay, render, retention, or completion
authority.

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

PreparedRunnerInvocation
  minted from CertifiedRuntimeSpec, VerifiedRunStream or GenesisContext, certified node metadata,
  verified typed config/input materialization, and per-attempt staging capabilities
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

`ReplayAuthority::new` and `ReplayBroker::from_run_stream` must not remain public production
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

The boundary type should be explicit. Domain runners receive a `PreparedRunnerInvocation` or an
equivalent private-field adapter input, not raw stream inspection, unverified artifact refs, or a
write-capable artifact store.

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

The split is:

- runtime materialization proves that persisted config/input artifacts match certified framework
  evidence and committed cell history;
- domain runner validation proves that the already materialized values are semantically valid for
  the domain and that external observations or side-effect protocol evidence are acceptable.

Generic evidence checks that can be expressed without domain semantics must not remain
runner-local merely because runners currently load their own artifacts.

## What Remains Special

The architecture intentionally preserves a small set of special boundaries:

- certified bundle verification at serialized boundaries;
- genesis context for the first batch;
- built-in framework runner resolution plus `BootstrapRun` genesis pre-execution middleware;
- artifact-store byte/evidence integrity;
- typed-store atomic commit/projection validation;
- read/resume/replay authority reconstruction from hostile persisted history.

The bootstrap first batch is special because there is no prior `RunStarted` stream authority. It is
not special because it bypasses middleware. It uses the same runtime mutation middleware through a
`GenesisContext`.

## Runtime Path Cleanup

The implementation must delete every alternative execution path that can mutate a run outside the
runtime mutation middleware. Synthetic test/migration/repair tooling is not a demoted runtime path;
it is non-execution infrastructure and must be isolated from production call graphs.

The cleanup rule is:

```text
if a path can append run events, admit run-store artifact evidence, persist mandatory runtime
artifacts, or derive lifecycle authority, it must go through runtime mutation middleware
```

The following must be removed as production APIs:

- standalone scheduler `start_run` logic that builds `RunStarted` from caller-assembled evidence;
- public constructors for scheduler-owned lifecycle payloads or framework lifecycle outputs that
  can be reached by user/domain runners;
- standalone scheduler `complete_run` logic that appends `RunCompleted` without a `CompleteRun`
  framework state;
- standalone retention projection append logic that is callable without
  `ProjectRetentionManifest`;
- app-side public-output receipt persistence or retention-manifest persistence;
- transport runner artifact sinks that expose `put_verified_artifact`;
- production store paths that call `record_artifact_evidence` before a later event append, or any
  wrapper around that split;
- replay, render, resume, or retention constructors that accept raw stream/status DTOs instead of
  sealed `VerifiedRunStream`-based authority;
- positive conformance helpers that manually persist spec/config/public-output artifacts or append
  lifecycle events outside the same app-agnostic runtime/framework primitives used by production.

Synthetic write APIs are allowed only as non-execution tools or fixtures, never as alternate
runtime paths:

- `#[cfg(test)]` fixtures;
- corruption and tamper tests;
- migration or repair tooling;
- storage contract tests that intentionally validate invalid or partial states.

Those tools and fixtures do not mint execution authority. Their output must re-enter the system
through the same certified-spec and stream-verification boundaries as any other persisted history.

No positive app, CLI, REST, transport, runtime, or conformance path may teach or preserve a
second runtime model.

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
- downstream code cannot construct `PreparedRunnerInvocation` from raw stream or artifact evidence;
- production runners cannot name or receive a write-capable artifact-store trait;
- user/domain runners cannot construct scheduler-owned lifecycle payloads or framework lifecycle
  outputs;
- replay authority cannot be constructed from a hash-only spec envelope.

Negative and corruption tests should cover:

- serialized specs with forged framework lifecycle variants or invalid lifecycle ordering;
- standalone `RunCompleted`, `RetentionManifestProjected`, or `RetentionRefsAppended` commits that
  are not derived from the matching sealed framework state terminal evidence;
- mismatched staged artifact digest, byte length, role, schema, semantic type, producer, attempt, or
  side-effect invocation epoch;
- admitted run-store artifact evidence without a referencing commit;
- tampered stored spec/certificate artifacts before resume, replay, public-output reads, or
  retention projection;
- replay or public-output construction from raw stream/status inspection rather than sealed read
  authority.

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

### Split Store Mutation APIs

This preserves a second authority model. Production code must not choose between split
`record_artifact_evidence` / `append_typed_run_commit` calls and prepared commits. The architecture
requires the breaking API cleanup: `append_prepared_typed_commit(PreparedTypedCommit)` is the
only execution mutation path. Synthetic evidence insertion is confined to non-execution
test/migration/repair surfaces that cannot be called by runtime scheduling, app services,
transport runners, positive conformance, CLI, or REST.

## Implementation Sequence

Prefer small, reviewable commits:

1. Define the runtime mutation middleware boundary and staged artifact model.
2. Replace production typed-store mutation with
   `append_prepared_typed_commit(PreparedTypedCommit)` in in-memory and durable stores.
3. Convert public-output receipt persistence to staged framework-runner output.
4. Replace production transport artifact write sinks with staged artifacts/handles.
5. Add `ProjectRetentionManifest` and move retention projection into a sealed framework state.
6. Add `CompleteRun` and derive `RunCompleted` from its sealed framework output.
7. Add `BootstrapRun`, `GenesisContext`, `PreparedRunLaunch`, and the bootstrap genesis middleware
   entrypoint that emits the same prepared-commit type as ordinary state execution.
8. Move launch/read/replay authority checks behind sealed runtime/replay constructors.
9. Update app, CLI, REST, conformance, and positive integration paths to use the unified
   authority APIs.
10. Delete split store mutation paths from execution; keep only explicit non-execution
    test/migration/repair fixtures where synthetic history is required.
11. Add the architectural coverage test proving that every executed state used runtime mutation
    middleware and that every referenced artifact was admitted with the same commit.

## Appendix A: Recommended Commit Plan

This is an implementation guide for the engineering team. Treat each step as one reviewable
commit. Do not preserve backward compatibility for replaced execution APIs: no deprecated aliases,
no compatibility shims, no dual production paths, and no temporary public wrappers that allow old
callers to keep mutating runs. A step is complete only when old production call sites either use
the new authority boundary or no longer compile.

- [x] Commit 1: `store: replace execution commits with prepared typed commits`
  - [x] Add the private-field `PreparedTypedCommit` authority type.
  - [x] Replace production store mutation with
    `append_prepared_typed_commit(PreparedTypedCommit)`.
  - [x] Remove `record_artifact_evidence` and direct `append_typed_run_commit` from
    execution-facing store traits.
  - [x] Move synthetic evidence/history construction to a separate non-execution test/tool surface
    that runtime, app, CLI, REST, transports, and positive conformance cannot call.
  - [x] Update in-memory and durable stores in the same commit so there is no split production
    store contract left behind.

- [x] Commit 2: `runtime: introduce mutation middleware commit builder`
  - [x] Add the runtime-owned builder that converts sealed execution intent into
    `PreparedTypedCommit`.
  - [x] Move existing terminal payload validation, precondition assembly, required artifact
    admission, and retention-ref derivation behind that builder.
  - [x] Ensure user/domain code cannot construct `PreparedTypedCommit` or scheduler-owned payloads.

- [x] Commit 3: `runtime: add staged artifact model`
  - [x] Replace runner-returned artifact evidence-only outputs with staged artifacts or sealed
    staged handles.
  - [x] Bind each staged artifact to run id, node id, attempt id, role, binding kind,
    schema/semantic identity, producer evidence, digest, and byte length.
  - [x] Ensure failed commits can leave only orphan artifact-store bytes, never admitted run-store
    evidence.

- [x] Commit 4: `public-output: stage receipt artifacts in framework middleware`
  - [x] Make the sealed `PublicOutputRender` runner return staged receipt bytes or a sealed handle.
  - [x] Persist/promote receipt bytes only from runtime middleware before the prepared commit.
  - [x] Delete app-side public-output receipt repair from the normal path.

- [x] Commit 5: `transports: remove write-capable artifact sinks from runners`
  - [x] Remove production `put_artifact`, `put_verified_artifact`, and equivalent sink capabilities
    from transport runner inputs.
  - [x] Convert proof, portfolio, and EVM DCV runners to return staged artifacts or sealed handles
    for fact, output, diagnostic, side-effect intent, receipt, and confirmation bytes.
  - [x] Keep artifact-store writes available only through runtime-owned staging/finalization.

- [x] Commit 6: `runtime: add prepared runner invocation materialization`
  - [x] Add `PreparedRunnerInvocation` or an equivalent private-field adapter input.
  - [x] Move generic config/input artifact checks before runner invocation: config refs, digest,
    length, media/schema ids, input cell terminal evidence, producer role, and committed evidence.
  - [x] Leave domain semantic validation, hostile external-data validation, side-effect protocol
    checks, and replay-specific external evidence checks in domain runners.

- [x] Commit 7: `framework: certify lifecycle framework nodes`
  - [x] Extend framework node certification for `BootstrapRun`, `ProjectRetentionManifest`, and
    `CompleteRun`.
  - [x] Reject serialized forged lifecycle variants, invalid framework descriptors, invalid
    lifecycle ordering, and framework-only payload permissions in certification.
  - [x] Repeat runtime framework-node contract checks when constructing `CertifiedRuntimeSpec`.

- [x] Commit 8: `retention: execute manifest projection as a framework state`
  - [x] Add `ProjectRetentionManifest` runtime execution using sealed runtime projection input.
  - [x] Build and stage the retention manifest from the authoritative pre-projection stream.
  - [x] Commit `StateAttemptStarted`, `CellProduced`, `StateAttemptCompleted`,
    `RetentionManifestProjected`, and `RetentionRefsAppended` through
    `append_prepared_typed_commit`.
  - [x] Delete standalone retention projection append APIs from execution.

- [x] Commit 9: `completion: derive run completion from completer state`
  - [x] Add `CompleteRun` as the final lifecycle framework state.
  - [x] Commit its attempt lifecycle and `RunCompleted` in the same prepared commit.
  - [x] Delete scheduler-only `complete_run` execution paths.
  - [x] Enforce `PublicOutputRender -> ProjectRetentionManifest -> CompleteRun` as the certified
    lifecycle tail.

- [x] Commit 10: `bootstrap: execute run start through genesis middleware`
  - [x] Add `BootstrapRun`, `GenesisContext`, and `PreparedRunLaunch`.
  - [x] Normalize bootstrap as a sealed framework state with certified node identity, attempt
    lifecycle, receipt cell, artifact bindings, and middleware-owned commit.
  - [x] Keep the only genesis-specific behavior in pre-execution middleware:
    `RequiredRunState::Absent`, launch artifact materialization, and `RunStarted` ordinal 0.
  - [x] Delete standalone scheduler `start_run` execution paths and caller-assembled
    `RunStartEvidence`.

- [x] Commit 11: `runtime: seal lifecycle payload authority`
  - [x] Make scheduler-owned lifecycle payloads and framework lifecycle outputs unconstructable by
    user/domain runners.
  - [x] Require runtime middleware to derive `RunStarted`, `RunCompleted`,
    `RetentionManifestProjected`, `RetentionRefsAppended`, and attempt lifecycle payloads.
  - [x] Add compile-fail coverage for lifecycle payload construction outside the runtime boundary.

- [x] Commit 12: `replay: require sealed replay read authority`
  - [x] Replace public production replay constructors that accept raw stream/spec evidence with
    `ReplayReadAuthority`.
  - [x] Mint replay authority only from certifier-backed spec authority, `VerifiedRunStream`, and
    retained evidence from committed stream/projection history.
  - [x] Ensure replay cannot construct live capabilities.

- [x] Commit 13: `public-output: require sealed read authority`
  - [x] Mint `PublicOutputReadAuthority` only after certified spec/certificate verification,
    `VerifiedRunStream` validation, projection rebuild, and artifact evidence checks.
  - [x] Ensure rendered JSON and raw status/stream DTOs cannot authorize another render, resume, or
    replay.

- [x] Commit 14: `app cli rest: migrate positive paths to unified authority`
  - [x] Update app, CLI, REST, and domain start flows to call only certified authority,
    `PreparedRunLaunch`, runtime middleware, and prepared store commit APIs.
  - [x] Remove app-owned mandatory runtime artifact persistence from normal execution.
  - [x] Ensure generic and domain starts, resume, replay, public-output reads, retention, and
    completion cannot call deleted execution surfaces.

- [x] Commit 15: `conformance: remove second runtime model from positive tests`
  - [x] Update positive conformance and integration helpers to use the same app-agnostic
    runtime/framework primitives as production.
  - [x] Keep direct store mutation only in explicitly named negative, corruption, migration, repair,
    or low-level storage contract fixtures.
  - [x] Ensure synthetic fixture output re-enters through certified-spec and `VerifiedRunStream`
    authority constructors before it can influence replay/read/runtime behavior.

- [x] Commit 16: `tests: add no-second-authority coverage`
  - [x] Add the representative architectural coverage test with `BootstrapRun`, one ordinary state,
    `PublicOutputRender`, `ProjectRetentionManifest`, and `CompleteRun`.
  - [x] Assert every executable unit appears as an attempt, every referenced artifact was staged
    through middleware, and every artifact evidence row was admitted in the same commit that first
    referenced it.
  - [x] Add negative tests for forged framework lifecycle nodes, standalone lifecycle events,
    mismatched staged artifact bindings, orphan run-store evidence, tampered spec/certificate
    artifacts, and raw stream/status replay or public-output construction.

- [x] Commit 17: `docs: update contracts after execution api replacement`
  - [x] Update `docs/design.md`, `docs/architecture.md`, runtime/replay/store READMEs, CLI docs,
    and REST docs to describe the single execution mutation path.
  - [x] Remove wording that implies compatibility with split evidence admission, app-owned runtime
    artifact persistence, scheduler lifecycle transitions, or write-capable runner artifact sinks.
  - [x] Document synthetic store mutation only as non-execution test/migration/repair tooling.

## Acceptance Criteria

The architecture is corrected when:

- all lifecycle work is represented as sealed framework nodes or the sealed bootstrap genesis
  batch;
- `BootstrapRun` has normal certified state identity, attempt lifecycle, and receipt evidence, while
  `RunStarted` is committed only through `PreparedRunLaunch` and the bootstrap genesis middleware;
- public-output receipt bytes are persisted by runtime middleware, not app repair code;
- retention projection is a sealed framework node using sealed runtime projection input;
- retention projection runs before terminal completion, and `RunCompleted` is the final lifecycle
  event derived from `CompleteRun`, not scheduler-only completion;
- production runners have no write-capable artifact-store traits;
- `ErasedRunnerOutput` or its replacement carries staged artifacts or sealed handles, not only
  evidence;
- production store mutation is only `append_prepared_typed_commit(PreparedTypedCommit)`, admitting
  artifact evidence and event payloads atomically;
- orphan artifact bytes are never treated as run authority;
- `record_artifact_evidence` and direct `append_typed_run_commit` are absent from positive
  app/transport/CLI/REST/runtime/conformance paths and have no execution wrapper;
- replay authority requires certifier-backed authority plus committed retained evidence;
- public-output read authority is minted only from verified certified authority and validated run
  stream evidence;
- raw status/stream DTOs cannot be passed where runtime, replay, public-output, or retention
  authority is required;
- typed config/input materialization reaches runners through a sealed prepared invocation boundary,
  with generic artifact/evidence checks owned by runtime rather than runner convention;
- forged framework lifecycle nodes, standalone scheduler-owned lifecycle events, mismatched staged
  artifact bindings, and orphan run-store evidence are rejected by tests and store/runtime
  validation;
- an architectural coverage test proves that every executable node in a representative certified
  run executed as a state attempt and used runtime mutation middleware for persistence;
- all former scheduler/app/transport alternative runtime mutation paths are deleted from execution;
- synthetic store mutation is confined to explicit non-execution test/migration/corruption
  fixtures;
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
