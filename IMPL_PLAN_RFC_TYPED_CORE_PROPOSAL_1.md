# Implementation Plan: RFC Typed Core Proposal 1

Status: implementation checklist

Source RFCs:

- `RFC_STATE_OPS_PROBLEM.md`
- `RFC_TYPED_CORE_PROPOSAL_1.md`

This plan is intentionally breaking. Do not preserve the old dynamic DAG model through compatibility
facades, legacy allowlists, or semantic wrappers around `PlannedOp`, `PortKey`, `DynContext`,
`StateGraph`, `DependencyEdge`, or generic `IoProvider`.

The target end state is a clean typed kernel, typed scheduler, typed store/replay path, typed
workflow ports, and deletion or hard isolation of old dynamic semantic APIs.

## Implementation Rules

- [ ] Keep each commit reviewable and focused on one logical outcome.
- [ ] Commit subjects must be lower case.
- [ ] Prefer deleting obsolete semantic APIs over adapting them.
- [ ] The new `crates/kernel/*` crates must never depend on `crates/machine`, `crates/sdk`,
      domain crates, or binaries.
- [ ] Breaking API, CLI, persisted-format, and workspace changes are allowed when they move the repo
      toward the RFC contract.
- [ ] Update docs in the same commit that introduces or removes a public contract.
- [ ] If a commit cannot keep the full workspace green during the rewrite, mark it explicitly in the
      commit body and restore green status by the next milestone gate. Do not merge a milestone while
      it is knowingly red.
- [ ] Default verification before publishing implementation commits: `nix run .#check`,
      `nix run .#test`, and `nix run .#ci -- --mode full --summary`.

## Milestone Gates

- [ ] `typed-kernel-contract`: typed authoring, descriptor, spec, event-schema, and certification
      contracts exist and reject the six problem classes.
- [ ] `typed-certified-slice`: serial typed runtime executes a reference certified workflow with
      typed store, typed events, replay/resume, public outputs, retention, and deterministic local
      side-effect recovery.
- [ ] `proof-implementation-conformance`: proof backend ports satisfy typed fact, submission,
      receipt, confirmation, output, and replay contracts.
- [ ] `portfolio-typed-port`: portfolio uses typed fanout/fanin, stable domain keys, typed public
      outputs, and no old SDK semantic imports.
- [ ] `evm-dcv-typed-port`: EVM deploy/configure/validate uses typed lifecycle, typed side effects,
      replay without reapply, and protected transaction secrecy.
- [ ] `dynamic-core-removal`: old dynamic semantic crates/APIs are deleted or isolated so they cannot
      submit, resume, or influence certified typed runs.

## Commit Plan

### Commit 01: `docs: add typed core implementation plan`

- [ ] Add this implementation plan.
- [ ] Link it from the RFC or a planning issue if desired.
- [ ] Validation: `git diff --check`.

### Commit 02: `workspace add typed kernel crate skeleton`

- [ ] Add workspace members:
      `crates/kernel/ids`,
      `crates/kernel/canonical`,
      `crates/kernel/values`,
      `crates/kernel/effects`,
      `crates/kernel/capabilities`,
      `crates/kernel/program`,
      `crates/kernel/program-derive`,
      `crates/kernel/spec`,
      `crates/kernel/certify`,
      `crates/kernel/events`,
      `crates/kernel/test-support`.
- [ ] Add `#![warn(missing_docs)]` to library crates.
- [ ] Keep all crates dependency-minimal and domain-free.
- [ ] Add placeholder READMEs stating RFC authority and crate ownership.
- [ ] Validation: `cargo metadata --no-deps`, `nix run .#check`.

### Commit 03: `check enforce typed kernel crate dag`

- [ ] Add or update the `crate-dag` check so `crates/kernel/*` cannot depend on:
      `crates/machine`, `crates/sdk`, `bin/*`, domain crates, old ops, or old states.
- [ ] Add denied dependency tests for the new kernel crates.
- [ ] Add a CI summary key: `kernel_crates_present`.
- [ ] Validation: `nix run .#check`.

### Commit 04: `kernel ids add strong identity types`

- [ ] Implement category-branded identity primitives in `mfm-ids`.
- [ ] Add typed ids for semantic types, schemas, states, capabilities, adapters, operations,
      descriptors, specs, nodes, cells, scopes, seeds, attempts, runs, events, artifacts, and
      content digests.
- [ ] Keep kind and version fields separate.
- [ ] Add checked constructors and grammar validation.
- [ ] Add golden tests for accepted/rejected identity strings.
- [ ] Validation: `nix run .#test`.

### Commit 05: `kernel canonical add canonical json contract`

- [ ] Implement canonical JSON bytes and digest wrappers in `mfm-canonical`.
- [ ] Reject floats, duplicate object keys, unsupported number forms, and non-canonical decimal
      strings.
- [ ] Add canonical digest test vectors for objects, maps, arrays, bytes, decimals, and rejected
      values.
- [ ] Validation: `nix run .#test`.

### Commit 06: `kernel values add schema descriptors and value traits`

- [ ] Implement `MfmValue`, `MfmConfig`, public-output descriptor primitives, and schema descriptor
      identity/audit split in `mfm-values`.
- [ ] Implement framework-owned generic constructors such as `MaybeValue<T>` and `ArtifactRef<T>`.
- [ ] Add secret/no-float persisted-surface policy types.
- [ ] Add descriptor golden tests.
- [ ] Validation: `nix run .#test`.

### Commit 07: `kernel derive add value config output derives`

- [ ] Implement derives in `mfm-program-derive` for `MfmValue`, `MfmConfig`, `StateInput`,
      `OperationOutput`, and `PublicOutputs`.
- [ ] Reject unsupported serde attributes, floats, raw `serde_json::Value`, `HashMap`, skipped
      fields, custom serializers, and known secret wrappers.
- [ ] Add trybuild pass/fail fixtures for downstream-crate derive use.
- [ ] Add manual impl rejection fixtures for domain crates.
- [ ] Validation: `nix run .#test`.

### Commit 08: `check block manual persisted trait impls`

- [ ] Add source-boundary checks that reject manual `impl MfmValue`, `impl MfmConfig`, and manual
      public-output impls outside framework allowlists.
- [ ] Add CI summary key: `manual_value_config_output_impls_rejected`.
- [ ] Validation: `nix run .#check`.

### Commit 09: `kernel effects add sealed effect markers`

- [ ] Implement sealed `Pure`, `ReadExternal`, `ManagedPlatformWrite`, and `ApplySideEffect`
      markers in `mfm-effects`.
- [ ] Add effect kind ids and versions where needed by descriptors.
- [ ] Ensure effect markers are framework-owned, not downstream extension traits.
- [ ] Validation: `nix run .#test`.

### Commit 10: `kernel capabilities add role checked capability sets`

- [ ] Implement capability descriptors, roles, `CapabilitySpec`, `CapabilitySet`, and
      `CapabilitySetFor<E>` in `mfm-capabilities`.
- [ ] Enforce v1 rules:
      pure gets no caps,
      read gets read/support only,
      managed-write gets platform persistence/output only,
      side-effect gets exactly one external mutation authority.
- [ ] Add trybuild failures for invalid capability roles.
- [ ] Validation: `nix run .#test`.

### Commit 11: `kernel program add handles roots and seeds`

- [ ] Implement invariant branded `Handle<'program, 'scope, T>`.
- [ ] Implement `build_root`, `RootBuilder`, `ScopeBuilder`, root seed specs, and public-output
      binding.
- [ ] Make handle constructors private and non-forgeable.
- [ ] Add trybuild failures for forged handles, escaped handles, and runtime values used as seeds.
- [ ] Validation: `nix run .#test`.

### Commit 12: `kernel program add scopes and bridge evidence`

- [ ] Implement root/child scope builders.
- [ ] Split persisted `BridgeRef` from live private `BridgeEvidence`.
- [ ] Implement bridge session validation and bridge node spec emission.
- [ ] Add trybuild failures for child/sibling handle mixing without bridge evidence.
- [ ] Add certification fixture for forged or stale bridge refs.
- [ ] Validation: `nix run .#test`.

### Commit 13: `kernel program add state inputs and bindings`

- [ ] Implement `StateInput`, `InputBindingSpec`, `InputBindingNode`, `IntoStateInput`, tuples,
      vectors, non-empty handles, optional values, artifact refs, and derive-backed input structs.
- [ ] Add value lineage refs to input cell bindings.
- [ ] Reject raw JSON, context keys, `OutputCellId`, dynamic erased inputs, and empty non-empty
      inputs.
- [ ] Validation: `nix run .#test`.

### Commit 14: `kernel program add state registry authority`

- [ ] Implement `StateSpec`, public effect-specific state traits, `RegisteredState<S>`, and private
      state registration evidence.
- [ ] Make `ScopeBuilder::state` resolve through a builder-owned registry or registered token.
- [ ] Add tests proving unregistered states cannot be planned or lowered.
- [ ] Add CI summary key: `registered_state_required`.
- [ ] Validation: `nix run .#test`.

### Commit 15: `kernel program add operation registry authority`

- [ ] Implement `Operation`, `RegisteredOperation<O>`, private operation registration evidence, and
      operation descriptors.
- [ ] Make `ScopeBuilder::call` resolve through an operation registry or registered token.
- [ ] Record operation lineage frames in the typed program.
- [ ] Add tests proving unregistered operations cannot be called.
- [ ] Add CI summary key: `registered_operation_required`.
- [ ] Validation: `nix run .#test`.

### Commit 16: `kernel program add stable ids and value lineage`

- [ ] Implement stable author keys and `StableDomainKey`.
- [ ] Implement `ValueLineageRef`, `ValueLineage`, domain key refs, and lineage transform policy.
- [ ] Derive deterministic scope, operation instance, node, cell, seed, input binding, config ref,
      operation lineage, and value lineage digests.
- [ ] Add golden vectors for stable ids and lineage.
- [ ] Add mismatch certification fixtures for same-type same-scope wrong lineage.
- [ ] Validation: `nix run .#test`.

### Commit 17: `kernel spec add v1 typed execution spec`

- [ ] Implement `mfm-spec::v1::TypedExecutionSpec` and `CertifiedSpecEnvelope`.
- [ ] Include scopes, seeds, descriptor identities, config refs, nodes, cells, public outputs,
      framework bridge nodes, public-output render nodes, planning lineage, and value lineage.
- [ ] Add canonical spec hash generation and golden fixtures.
- [ ] Reject obsolete `state_program`/`outputs` sketch shapes.
- [ ] Validation: `nix run .#test`.

### Commit 18: `kernel events add v1 event schemas`

- [ ] Implement `mfm-events::v1` closed kernel event enum and payload structs.
- [ ] Include `RunStarted`, attempt events, fact/artifact events, cell events, side-effect events,
      public-output events, run completion, and retention events.
- [ ] Use `run_started_v1_present` in summary output, never `run_started_v2_present`.
- [ ] Add schema descriptor golden fixtures for all event payloads.
- [ ] Validation: `nix run .#test`.

### Commit 19: `kernel certify add typed spec certification`

- [ ] Implement certification over typed program drafts and `v1::TypedExecutionSpec`.
- [ ] Verify registered state/operation descriptors, config refs, input bindings, cells, effects,
      capabilities, side-effect contracts, public outputs, stable ids, bridge nodes, lineage, and
      dynamic collection ordering.
- [ ] Reject all six problem taxonomy classes.
- [ ] Add CI summary keys:
      `invalid_topology_rejected`,
      `invalid_interface_wiring_rejected`,
      `invalid_semantic_transition_rejected`,
      `invalid_data_shape_rejected`,
      `invalid_data_meaning_rejected`,
      `invalid_terminal_shape_rejected`.
- [ ] Validation: `nix run .#test`.

### Commit 20: `ci add typed kernel contract gate`

- [ ] Add `typed-kernel-contract` to Nixfied check/test/summary wiring.
- [ ] Emit all required `typed-kernel-contract` summary keys from the RFC.
- [ ] Fail on missing, false, skipped, xfail, or zero required counts.
- [ ] Validation: `nix run .#check`, `nix run .#test`.

### Commit 21: `kernel store add typed commit contract`

- [ ] Add `crates/kernel/store` and `mfm-store`.
- [ ] Define typed commit API, commit keys, logical keys, preconditions, artifact evidence refs, and
      projection traits.
- [ ] Make commit-key lookup precede stale `expected_next_seq`.
- [ ] Ensure callers cannot forge sequence, ordinal, event id, envelopes, or projections.
- [ ] Add storage commit contract tests.
- [ ] Validation: `nix run .#test`.

### Commit 22: `storage postgres add typed run event store`

- [ ] Replace or rewrite `crates/storages/stream-store-postgres` around `mfm-store`.
- [ ] Add migrations for typed run events, commit keys, artifacts, cell projection, fact projection,
      side-effect projection, public-output projection, retention projection, and retention manifests.
- [ ] Enforce contiguous sequence, logical-key conflicts, required artifact preconditions, and
      projection rebuild.
- [ ] Validation: `nix run .#test`.

### Commit 23: `storage fs add typed artifact store`

- [ ] Rewrite `crates/storages/artifact-store-fs` as the first certified local artifact store.
- [ ] Store canonical bytes by digest and verify digest, byte length, media type, schema id,
      semantic id, producer node/seed, and artifact role.
- [ ] Enforce seed material persistence and missing seed artifact rejection.
- [ ] Validation: `nix run .#test`.

### Commit 24: `kernel runtime add serial typed scheduler`

- [ ] Add `crates/kernel/runtime` and `mfm-runtime`.
- [ ] Implement the serial typed scheduler over certified specs and erased runner bindings derived
      only from the spec.
- [ ] Implement deterministic runnable-node selection.
- [ ] Materialize typed inputs only from certified cells.
- [ ] Mint only certified capabilities.
- [ ] Validation: `nix run .#test`.

### Commit 25: `kernel runtime add non side effect recovery`

- [ ] Implement attempt recovery for pure, read, and managed-write states.
- [ ] Require terminal cell and `StateAttemptCompleted` in the same atomic terminal commit.
- [ ] Reuse committed facts for the same read attempt.
- [ ] Allow managed-write artifact re-stage only before typed commit authority.
- [ ] Add crash/restart fixtures for pure, read, and managed-write attempts.
- [ ] Validation: `nix run .#test`.

### Commit 26: `kernel runtime add side effect ledger`

- [ ] Implement side-effect phases:
      intent,
      claim,
      invocation prepared,
      invocation started,
      submission,
      receipt,
      confirmation,
      output,
      failure,
      ambiguity.
- [ ] Add `ClaimTakenOver`, claim generation, and claim fencing token logic.
- [ ] Reject stale owners, stale fencing tokens, duplicate submits, receipt without submission,
      confirmation without receipt, and takeover after invocation started.
- [ ] Use a deterministic local adapter fixture, not a live external service.
- [ ] Validation: `nix run .#test`.

### Commit 27: `kernel replay add replay brokers and verifiers`

- [ ] Add `crates/kernel/replay` and `mfm-replay`.
- [ ] Replay from stored certified spec as authority.
- [ ] Implement replay brokers that answer only from recorded facts, submissions, receipts,
      confirmations, artifacts, and typed evidence.
- [ ] Reject live capability requests, missing facts, mismatched hashes/schema ids, unsupported
      adapter versions, executable identity mismatch, and canonicalizer mismatch.
- [ ] Validation: `nix run .#test`.

### Commit 28: `kernel runtime add public outputs`

- [ ] Implement framework-owned public-output render node.
- [ ] Produce `PublicOutputReceipt`, `PublicOutputProduced`, and `RunCompleted(Completed)` only after
      public-output evidence exists.
- [ ] Make rendered JSON a cache, not authority.
- [ ] Add render failure/resume tests.
- [ ] Validation: `nix run .#test`.

### Commit 29: `kernel runtime add retention event sourcing`

- [ ] Implement staged retention refs and store-owned `RetentionRefsAppended`.
- [ ] Rebuild retention projection from run stream only.
- [ ] Implement append-only `RetentionManifestProjected` artifacts.
- [ ] Ensure local artifact GC refuses verified retained artifacts.
- [ ] Validation: `nix run .#test`.

### Commit 30: `ci add typed certified slice gate`

- [ ] Add the synthetic reference certified workflow.
- [ ] Exercise pure, read, managed-write, and deterministic local side-effect paths.
- [ ] Emit all `typed-certified-slice` summary keys from the RFC.
- [ ] Fail on missing seed material, missing public-output evidence, replay live-cap request, resume
      drift, ambiguity, duplicate submit, or incomplete retention projection.
- [ ] Validation: `nix run .#ci -- --mode full --summary`.

### Commit 31: `app wire typed runtime assembly`

- [ ] Rewrite `crates/app` as assembly only:
      input decoding,
      registry construction,
      store/artifact/capability selection,
      start/resume/replay dispatch,
      typed public-output rendering.
- [ ] Remove old SDK planning/runtime authority from app code.
- [ ] Validation: `nix run .#test`.

### Commit 32: `cli rewrite for typed public outputs`

- [ ] Rewrite `bin/cli` to launch, resume, replay, and render typed certified runs only.
- [ ] Preserve documented stable public JSON fields where still valid.
- [ ] Remove semantic `PortKey`, context snapshot, dynamic DAG, and old SDK command paths.
- [ ] Update `bin/cli/README.md`.
- [ ] Validation: `nix run .#test`.

### Commit 33: `rest api rewrite for typed runs`

- [ ] Rewrite `bin/rest-api` around typed run start/resume/replay and typed public outputs.
- [ ] Remove dynamic DAG and context-dataflow endpoints.
- [ ] Update `bin/rest-api/README.md`.
- [ ] Validation: `nix run .#test`.

### Commit 34: `proof port to typed contracts`

- [ ] Rewrite proof states and operations around typed facts, intent, idempotency input,
      submission, receipt, confirmation, output, public outputs, and replay verifier behavior.
- [ ] Remove proof dependency on old `crates/sdk`, `crates/machine`, context keys, and hand-authored
      edges.
- [ ] Add `proof-implementation-conformance` fixtures for enabled proof implementations.
- [ ] Validation: `nix run .#ci -- --mode full --summary`.

### Commit 35: `portfolio model align with typed values`

- [ ] Move stable portfolio value/config/public-output types onto derives.
- [ ] Add `StableDomainKey` types for source, subject, view, valuation, observation batch, and
      report keys.
- [ ] Ensure portfolio model/config crates do not depend on runtime, old machine, or old SDK crates.
- [ ] Validation: `nix run .#test`.

### Commit 36: `portfolio port tracker workflow`

- [ ] Rewrite portfolio tracker states and operations through typed builder APIs.
- [ ] Use typed fanout/fanin, `DomainKeyedHandles`, non-empty observations where required, value
      lineage, and typed public outputs.
- [ ] Remove `PortKey`, semantic JSON context dataflow, hand-authored dependency edges, and old SDK
      semantic imports.
- [ ] Add duplicate domain key, stable ordering, replay/resume, and CLI/API parity fixtures.
- [ ] Validation: `nix run .#ci -- --mode parity --summary`.

### Commit 37: `evm model align with typed lifecycle`

- [ ] Move EVM deploy/configure/validate domain values onto typed values/configs.
- [ ] Add typestate values such as deployed contract, configured contract, validation report, and
      existing configured contract refs.
- [ ] Ensure raw protected transactions never implement `MfmValue`, `MfmConfig`, or public-output
      traits.
- [ ] Validation: `nix run .#test`.

### Commit 38: `evm dcv port typed workflow`

- [ ] Rewrite deploy/configure/validate operation through typed lifecycle states.
- [ ] Make deploy/configure side-effect states with typed intent, idempotency input, submission,
      receipt, confirmation, and output.
- [ ] Make validate a read state with typed read caps.
- [ ] Add compile-fail fixture for validation before configuration.
- [ ] Add deterministic local EVM/reth crash-boundary tests if enabled by CI environment.
- [ ] Validation: `nix run .#ci -- --mode parity --summary`.

### Commit 39: `transports port typed capabilities`

- [ ] Rewrite transport crates as typed live/replay capability backends.
- [ ] Remove generic `IoProvider` request/response semantics.
- [ ] Ensure replay transports cannot mint mutation authority or fall back to live IO.
- [ ] Add adapter recovery conformance tests.
- [ ] Validation: `nix run .#test`.

### Commit 40: `delete old dynamic machine sdk crates`

- [ ] Remove `crates/machine`, `crates/machine-derive`, `crates/machine-test-support`, and
      `crates/sdk` from the workspace, or reduce `crates/sdk` to typed reexports only if still
      deliberately needed.
- [ ] Delete old `PlannedOp`, `PortKey`, `DynContext`, public `StateGraph`, `DependencyEdge`, and
      dynamic DAG authoring surfaces.
- [ ] Delete tests that assert old dynamic semantics.
- [ ] Replace any remaining imports with typed kernel APIs or remove the dependent feature.
- [ ] Validation: `nix run .#check`.

### Commit 41: `delete old dynamic ops and states`

- [ ] Remove or rewrite old op crates that still build `StateNode`, `DependencyEdge`, `PlannedOp`,
      `PortKey`, or JSON context dataflow.
- [ ] Remove or rewrite old state crates that read semantic inputs from `DynContext`.
- [ ] Preserve only typed state/op crates that satisfy the RFC boundaries.
- [ ] Validation: `nix run .#test`.

### Commit 42: `delete old dynamic storage and replay paths`

- [ ] Remove old stream/event/projection code that can act as semantic authority outside
      `mfm-store`.
- [ ] Remove independent authoritative side-effect streams.
- [ ] Keep any compatibility readers only as legacy inspection tools that cannot start, resume, or
      certify typed runs.
- [ ] Validation: `nix run .#test`.

### Commit 43: `docs rewrite architecture for typed core`

- [ ] Rewrite `docs/design.md` so typed certified specs are the normative execution contract.
- [ ] Rewrite `docs/architecture.md` around the typed crate DAG and typed runtime boundary.
- [ ] Rewrite `docs/ops-and-states.md` with the typed inventory.
- [ ] Update `docs/repo-map.md` and crate READMEs.
- [ ] Ensure docs no longer present old `DynContext`/`IoProvider`/dynamic DAG as authoritative.
- [ ] Validation: `nix run .#check`.

### Commit 44: `ci remove old semantic api escape hatches`

- [ ] Add source-boundary checks for forbidden old symbols:
      `PlannedOp`,
      `PortKey`,
      `DynContext`,
      public `StateGraph`,
      public `DependencyEdge`,
      generic `IoProvider`,
      context-key semantic dataflow.
- [ ] Add checks that old dynamic APIs cannot submit certified typed execution specs.
- [ ] Add summary key: `typed_boundary_firewall_passed`.
- [ ] Validation: `nix run .#check`.

### Commit 45: `ci finalize full typed core gate`

- [ ] Make `typed-kernel-contract` and `typed-certified-slice` mandatory in full CI.
- [ ] Make enabled proof, portfolio, and EVM typed-port gates mandatory.
- [ ] Ensure `nix run .#ci -- --mode full --summary` fails on missing, false, skipped, or xfail
      required keys.
- [ ] Validation: `nix run .#ci -- --mode full --summary`.

### Commit 46: `cleanup remove obsolete fixtures and docs`

- [ ] Remove stale fixtures, snapshots, migration notes, and README sections that describe the old
      semantic runtime as active.
- [ ] Keep explicit old-run operational policy only where needed for inspection or migration refusal.
- [ ] Ensure public docs explain that old dynamic runs are not silently migrated into certified typed
      runs.
- [ ] Validation: `nix run .#check`, `nix run .#test`.

### Commit 47: `release typed core rewrite`

- [ ] Run `nix run .#check`.
- [ ] Run `nix run .#test`.
- [ ] Run `nix run .#ci -- --mode audit --summary`.
- [ ] Run `nix run .#ci -- --mode parity --summary`.
- [ ] Run `nix run .#ci -- --mode full --summary`.
- [ ] Confirm summary includes:
      `typed-kernel-contract`,
      `typed-certified-slice`,
      `typed_boundary_firewall_passed`,
      `run_started_v1_present`,
      `seed_material_persisted`,
      `replay_live_cap_requests_count == 0`,
      `resume_drift_rejected`,
      `retention_projection_complete`.
- [ ] Confirm no typed-core crate depends on old dynamic crates.
- [ ] Confirm old dynamic semantic APIs are deleted or unable to submit/resume certified typed runs.

## Final Merge Checklist

- [ ] `RFC_TYPED_CORE_PROPOSAL_1.md` remains the typed-core authority or has been superseded by
      updated `docs/design.md` and `docs/architecture.md`.
- [ ] `docs/design.md` and `docs/architecture.md` no longer describe old dynamic DAG execution as
      authoritative.
- [ ] `crates/kernel/*` exists and follows the dependency DAG.
- [ ] Old semantic APIs cannot be imported by typed workflows.
- [ ] CLI and REST render typed public outputs, not context snapshots.
- [ ] Replay uses stored certified specs and replay adapters only.
- [ ] Side-effect retry never blindly resubmits after `InvocationStarted`.
- [ ] Retention authority is event-sourced from run events.
- [ ] Secrets cannot cross value/config/output/event/error boundaries.
- [ ] Full CI summary is green with no skipped or expected-failure required gates.
