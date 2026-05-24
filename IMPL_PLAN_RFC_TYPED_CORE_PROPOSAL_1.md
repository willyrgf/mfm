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

- [x] Add this implementation plan.
- [x] Link it from the RFC or a planning issue if desired.
- [x] Validation: `git diff --check`.

### Commit 02: `workspace add typed kernel crate skeleton`

- [x] Add workspace members:
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
- [x] Add `#![warn(missing_docs)]` to library crates.
- [x] Keep all crates dependency-minimal and domain-free.
- [x] Add placeholder READMEs stating RFC authority and crate ownership.
- [x] Validation: `cargo metadata --no-deps`, `nix run .#check`.

### Commit 03: `check enforce typed kernel crate dag`

- [x] Add or update the `crate-dag` check so `crates/kernel/*` cannot depend on:
      `crates/machine`, `crates/sdk`, `bin/*`, domain crates, old ops, or old states.
- [x] Add denied dependency tests for the new kernel crates.
- [x] Add a CI summary key: `kernel_crates_present`.
- [x] Validation: `nix run .#check`.

### Commit 04: `kernel ids add strong identity types`

- [x] Implement category-branded identity primitives in `mfm-ids`.
- [x] Add typed ids for semantic types, schemas, states, capabilities, adapters, operations,
      descriptors, specs, nodes, cells, scopes, seeds, attempts, runs, events, artifacts, and
      content digests.
- [x] Keep kind and version fields separate.
- [x] Add checked constructors and grammar validation.
- [x] Add golden tests for accepted/rejected identity strings.
- [x] Validation: `nix run .#test`.

### Commit 05: `kernel canonical add canonical json contract`

- [x] Implement canonical JSON bytes and digest wrappers in `mfm-canonical`.
- [x] Reject floats, duplicate object keys, unsupported number forms, and non-canonical decimal
      strings.
- [x] Add canonical digest test vectors for objects, maps, arrays, bytes, decimals, and rejected
      values.
- [x] Validation: `nix run .#test`.

### Commit 06: `kernel values add schema descriptors and value traits`

- [x] Implement `MfmValue`, `MfmConfig`, public-output descriptor primitives, and schema descriptor
      identity/audit split in `mfm-values`.
- [x] Implement framework-owned generic constructors such as `MaybeValue<T>` and `ArtifactRef<T>`.
- [x] Add secret/no-float persisted-surface policy types.
- [x] Add descriptor golden tests.
- [x] Validation: `nix run .#test`.

### Commit 07: `kernel derive add value config output derives`

- [x] Implement derives in `mfm-program-derive` for `MfmValue`, `MfmConfig`, `StateInput`,
      `OperationOutput`, and `PublicOutputs`.
- [x] Reject unsupported serde attributes, floats, raw `serde_json::Value`, `HashMap`, skipped
      fields, custom serializers, and known secret wrappers.
- [x] Add trybuild pass/fail fixtures for downstream-crate derive use.
- [x] Add manual impl rejection fixtures for domain crates.
- [x] Validation: `nix run .#test`.

### Commit 08: `check block manual persisted trait impls`

- [x] Add source-boundary checks that reject manual `impl MfmValue`, `impl MfmConfig`, and manual
      public-output impls outside framework allowlists.
- [x] Add CI summary key: `manual_value_config_output_impls_rejected`.
- [x] Validation: `nix run .#check`.

### Commit 09: `kernel effects add sealed effect markers`

- [x] Implement sealed `Pure`, `ReadExternal`, `ManagedPlatformWrite`, and `ApplySideEffect`
      markers in `mfm-effects`.
- [x] Add effect kind ids and versions where needed by descriptors.
- [x] Ensure effect markers are framework-owned, not downstream extension traits.
- [x] Validation: `nix run .#test`.

### Commit 10: `kernel capabilities add role checked capability sets`

- [x] Implement capability descriptors, roles, `CapabilitySpec`, `CapabilitySet`, and
      `CapabilitySetFor<E>` in `mfm-capabilities`.
- [x] Enforce v1 rules:
      pure gets no caps,
      read gets read/support only,
      managed-write gets platform persistence/output only,
      side-effect gets exactly one external mutation authority.
- [x] Add trybuild failures for invalid capability roles.
- [x] Validation: `nix run .#test`.

### Commit 11: `kernel program add handles roots and seeds`

- [x] Implement invariant branded `Handle<'program, 'scope, T>`.
- [x] Implement `build_root`, `RootBuilder`, `ScopeBuilder`, root seed specs, and public-output
      binding.
- [x] Make handle constructors private and non-forgeable.
- [x] Add trybuild failures for forged handles, escaped handles, and runtime values used as seeds.
- [x] Validation: `nix run .#test`.

### Commit 12: `kernel program add scopes and bridge evidence`

- [x] Implement root/child scope builders.
- [x] Split persisted `BridgeRef` from live private `BridgeEvidence`.
- [x] Implement bridge session validation and bridge node spec emission.
- [x] Add trybuild failures for child/sibling handle mixing without bridge evidence.
- [x] Add certification fixture for forged or stale bridge refs.
- [x] Validation: `nix run .#test`.

### Commit 13: `kernel program add state inputs and bindings`

- [x] Implement `StateInput`, `InputBindingSpec`, `InputBindingNode`, `IntoStateInput`, tuples,
      vectors, non-empty handles, optional values, artifact refs, and derive-backed input structs.
- [x] Add value lineage refs to input cell bindings.
- [x] Reject raw JSON, context keys, `OutputCellId`, dynamic erased inputs, and empty non-empty
      inputs.
- [x] Validation: `nix run .#test`.

### Commit 14: `kernel program add state registry authority`

- [x] Implement `StateSpec`, public effect-specific state traits, `RegisteredState<S>`, and private
      state registration evidence.
- [x] Make `ScopeBuilder::state` resolve through a builder-owned registry or registered token.
- [x] Add tests proving unregistered states cannot be planned or lowered.
- [x] Add CI summary key: `registered_state_required`.
- [x] Validation: `nix run .#test`.

### Commit 15: `kernel program add operation registry authority`

- [x] Implement `Operation`, `RegisteredOperation<O>`, private operation registration evidence, and
      operation descriptors.
- [x] Make `ScopeBuilder::call` resolve through an operation registry or registered token.
- [x] Record operation lineage frames in the typed program.
- [x] Add tests proving unregistered operations cannot be called.
- [x] Add CI summary key: `registered_operation_required`.
- [x] Validation: `nix run .#test`.

### Commit 16: `kernel program add stable ids and value lineage`

- [x] Implement stable author keys and `StableDomainKey`.
- [x] Implement `ValueLineageRef`, `ValueLineage`, domain key refs, and lineage transform policy.
- [x] Derive deterministic scope, operation instance, node, cell, seed, input binding, config ref,
      operation lineage, and value lineage digests.
- [x] Add golden vectors for stable ids and lineage.
- [x] Add mismatch certification fixtures for same-type same-scope wrong lineage.
- [x] Validation: `nix run .#test`.

### Commit 17: `kernel spec add v1 typed execution spec`

- [x] Implement `mfm-spec::v1::TypedExecutionSpec` and `CertifiedSpecEnvelope`.
- [x] Include scopes, seeds, descriptor identities, config refs, nodes, cells, public outputs,
      framework bridge nodes, public-output render nodes, planning lineage, and value lineage.
- [x] Add canonical spec hash generation and golden fixtures.
- [x] Reject obsolete `state_program`/`outputs` sketch shapes.
- [x] Validation: `nix run .#test`.

### Commit 18: `kernel events add v1 event schemas`

- [x] Implement `mfm-events::v1` closed kernel event enum and payload structs.
- [x] Include `RunStarted`, attempt events, fact/artifact events, cell events, side-effect events,
      public-output events, run completion, and retention events.
- [x] Use `run_started_v1_present` in summary output, never `run_started_v2_present`.
- [x] Add schema descriptor golden fixtures for all event payloads.
- [x] Validation: `nix run .#test`.

### Commit 19: `kernel certify add typed spec certification`

- [x] Implement certification over typed program drafts and `v1::TypedExecutionSpec`.
- [x] Verify registered state/operation descriptors, config refs, input bindings, cells, effects,
      capabilities, side-effect contracts, public outputs, stable ids, bridge nodes, lineage, and
      dynamic collection ordering.
- [x] Reject all six problem taxonomy classes.
- [x] Add CI summary keys:
      `invalid_topology_rejected`,
      `invalid_interface_wiring_rejected`,
      `invalid_semantic_transition_rejected`,
      `invalid_data_shape_rejected`,
      `invalid_data_meaning_rejected`,
      `invalid_terminal_shape_rejected`.
- [x] Validation: `nix run .#test`.

### Commit 20: `ci add typed kernel contract gate`

- [x] Add `typed-kernel-contract` to Nixfied check/test/summary wiring.
- [x] Emit all required `typed-kernel-contract` summary keys from the RFC.
- [x] Fail on missing, false, skipped, xfail, or zero required counts.
- [x] Validation: `nix run .#check`, `nix run .#test`.

### Commit 21: `kernel store add typed commit contract`

- [x] Add `crates/kernel/store` and `mfm-store`.
- [x] Define typed commit API, commit keys, logical keys, preconditions, artifact evidence refs, and
      projection traits.
- [x] Make commit-key lookup precede stale `expected_next_seq`.
- [x] Ensure callers cannot forge sequence, ordinal, event id, envelopes, or projections.
- [x] Add storage commit contract tests.
- [x] Validation: `nix run .#test`.

### Commit 22: `storage postgres add typed run event store`

- [x] Replace or rewrite `crates/storages/stream-store-postgres` around `mfm-store`.
- [x] Add migrations for typed run events, commit keys, artifacts, cell projection, fact projection,
      side-effect projection, public-output projection, retention projection, and retention manifests.
- [x] Enforce contiguous sequence, logical-key conflicts, required artifact preconditions, and
      projection rebuild.
- [x] Validation: `nix run .#test`.

### Commit 23: `storage fs add typed artifact store`

- [x] Rewrite `crates/storages/artifact-store-fs` as the first certified local artifact store.
- [x] Store canonical bytes by digest and verify digest, byte length, media type, schema id,
      semantic id, producer node/seed, and artifact role.
- [x] Enforce seed material persistence and missing seed artifact rejection.
- [x] Validation: `nix run .#test`.

### Commit 24: `kernel runtime add serial typed scheduler`

- [x] Add `crates/kernel/runtime` and `mfm-runtime`.
- [x] Implement the serial typed scheduler over certified specs and erased runner bindings derived
      only from the spec.
- [x] Implement deterministic runnable-node selection.
- [x] Materialize typed inputs only from certified cells.
- [x] Mint only certified capabilities.
- [x] Validation: `nix run .#test`.

### Commit 25: `kernel runtime add non side effect recovery`

- [x] Implement attempt recovery for pure, read, and managed-write states.
- [x] Require terminal cell and `StateAttemptCompleted` in the same atomic terminal commit.
- [x] Reuse committed facts for the same read attempt.
- [x] Allow managed-write artifact re-stage only before typed commit authority.
- [x] Add crash/restart fixtures for pure, read, and managed-write attempts.
- [x] Validation: `nix run .#test`.

### Commit 26: `kernel runtime add side effect ledger`

- [x] Implement side-effect phases:
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
- [x] Add `ClaimTakenOver`, claim generation, and claim fencing token logic.
- [x] Reject stale owners, stale fencing tokens, duplicate submits, receipt without submission,
      confirmation without receipt, and takeover after invocation started.
- [x] Use a deterministic local adapter fixture, not a live external service.
- [x] Validation: `nix run .#test`.

### Commit 27: `kernel replay add replay brokers and verifiers`

- [x] Add `crates/kernel/replay` and `mfm-replay`.
- [x] Replay from stored certified spec as authority.
- [x] Implement replay brokers that answer only from recorded facts, submissions, receipts,
      confirmations, artifacts, and typed evidence.
- [x] Reject live capability requests, missing facts, mismatched hashes/schema ids, unsupported
      adapter versions, executable identity mismatch, and canonicalizer mismatch.
- [x] Validation: `nix run .#test`.

### Commit 28: `kernel runtime add public outputs`

- [x] Implement framework-owned public-output render node.
- [x] Produce `PublicOutputReceipt`, `PublicOutputProduced`, and `RunCompleted(Completed)` only after
      public-output evidence exists.
- [x] Make rendered JSON a cache, not authority.
- [x] Add render failure/resume tests.
- [x] Validation: `nix run .#test`.

### Commit 29: `kernel runtime add retention event sourcing`

- [x] Implement staged retention refs and store-owned `RetentionRefsAppended`.
- [x] Rebuild retention projection from run stream only.
- [x] Implement append-only `RetentionManifestProjected` artifacts.
- [x] Ensure local artifact GC refuses verified retained artifacts.
- [x] Validation: `nix run .#test`.

### Commit 30: `ci add typed certified slice gate`

- [x] Add the synthetic reference certified workflow.
- [x] Exercise pure, read, managed-write, and deterministic local side-effect paths.
- [x] Emit all `typed-certified-slice` summary keys from the RFC.
- [x] Fail on missing seed material, missing public-output evidence, replay live-cap request, resume
      drift, ambiguity, duplicate submit, or incomplete retention projection.
- [x] Validation: `nix run .#ci -- --mode full --summary`.

### Commit 31: `app wire typed runtime assembly`

- [x] Rewrite `crates/app` as assembly only:
      input decoding,
      registry construction,
      store/artifact/capability selection,
      start/resume/replay dispatch,
      typed public-output rendering.
- [x] Remove old SDK planning/runtime authority from app code.
- [x] Validation: `nix run .#test`.

### Commit 32: `cli rewrite for typed public outputs`

- [x] Rewrite `bin/cli` to launch, resume, replay, and render typed certified runs only.
- [x] Preserve documented stable public JSON fields where still valid.
- [x] Remove semantic `PortKey`, context snapshot, dynamic DAG, and old SDK command paths.
- [x] Update `bin/cli/README.md`.
- [x] Validation: `nix run .#test`.

### Commit 33: `rest api rewrite for typed runs`

- [x] Rewrite `bin/rest-api` around typed run start/resume/replay and typed public outputs.
- [x] Remove dynamic DAG and context-dataflow endpoints.
- [x] Update `bin/rest-api/README.md`.
- [x] Validation: `nix run .#test`.

### Commit 34: `proof port to typed contracts`

- [x] Rewrite proof states and operations around typed facts, intent, idempotency input,
      submission, receipt, confirmation, output, public outputs, and replay verifier behavior.
- [x] Remove proof dependency on old `crates/sdk`, `crates/machine`, context keys, and hand-authored
      edges.
- [x] Add `proof-implementation-conformance` fixtures for enabled proof implementations.
- [x] Validation: `nix run .#ci -- --mode full --summary`.

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
