# RFC: certified saga vertical slice for MFM

Status: ready for engineering planning. Revised after independent architecture review; this
revision supersedes the v1 scope in `DESIGN_ACDC.md` where they disagree.

Source documents:

- `PROBLEM_ACDC.md`
- `DESIGN_ACDC.md`
- `docs/design.md`
- `docs/architecture.md`
- `docs/code-quality.md`

## Summary

MFM should implement a narrow certified saga vertical slice before attempting broader
Stonebraker-style AC/DC claims.

The target is not full AC/DC for arbitrary external systems. The target is platform-owned certified
saga semantics for external side effects:

1. a certified run-level saga policy;
2. side-effect-grade remedial mutations structurally linked to forward side effects;
3. a minimal-fact append-only stream: remedial ledger evidence, operator evidence, and one sealed
   terminal resolution;
4. runtime-owned transition from forward failure to remediation or manual resolution, derived
   deterministically from certified spec plus recorded facts;
5. store-owned projections and preconditions for remediation authority;
6. replay verification without live IO;
7. public status that reports semantic run modes and per-ledger obligation state.

Stronger AC/DC-style claims are allowed only where MFM owns the affected transactional resource or
where replay can verify a certified domain proof from typed evidence. Core v1 makes no such claim.

## Non-Goals

- Do not claim full AC/DC for arbitrary external systems.
- Do not add a new `ApplyCompensation` effect class in v1.
- Do not add per-node failure directive overrides in v1. Saga policy is run-level.
- Do not add obligation or manual-resolution ID namespaces. Obligation identity is the forward
  side-effect node and its ledger key.
- Do not record derivable decisions as stream events. Directive selection, obligation open/close,
  and run mode are projections of certified spec plus recorded facts, not appendable payloads.
- Do not add dynamic remediation callbacks, closures, CLI handlers, or app-level policy hooks.
- Do not make every state implement compensation methods.
- Do not implement generic commutativity, escrow, predicate isolation, resource claims, or chain
  reorg semantics in the kernel.
- Do not preserve the old absent/started/completed public status model as the user-facing contract.
- Do not hide compensation as ordinary user states that runtime/store/replay cannot recognize.
- Do not define run cancellation semantics in v1. Cancellation is explicitly deferred and removed
  from the terminal vocabulary until it is designed against remediation.

## Engineering Constraints

These constraints are part of the RFC, not implementation preferences.

1. Prefer unrepresentable invalid states over runtime rejection, and prefer structural
   unrepresentability over typestate ceremony.

   Invalid saga configurations should be impossible to express in the spec data model itself where
   practical, not merely impossible to build through the public API. A remediation node that is
   structurally outside the forward graph cannot be forward-scheduled even by hostile persisted
   bytes. Certification still verifies lowered specs, but every invariant that the data shape can
   carry is one certification rule and one forgery surface deleted.

2. Record facts, derive decisions.

   The append-only stream records only non-derivable facts: attempt lifecycle, ledger IO evidence,
   operator evidence, and sealed terminal resolution. Everything deterministic from certified spec
   plus those facts — selected directive, obligation state, run mode — is a rebuildable projection.
   A decision that is never written cannot be forged and never needs a replay forgery check.

3. Keep the public API and LOC growth narrow.

   V1 adds only: run-level saga policy, a linked-compensation authoring surface, ledger purpose,
   one operator evidence event, one sealed terminal lifecycle node, semantic run mode, and the
   projections behind public status. No generic callback systems, no broad new traits, no duplicate
   side-effect machinery, no speculative correctness classes.

4. Break compatibility deliberately.

   Backward compatibility is not a goal. If old public status, persisted spec shape, event schema,
   or terminal outcome vocabulary conflicts with the certified saga model, change it cleanly and
   update docs/tests in the same PR. No compatibility shims for flawed semantics.

5. Divide work by semantic boundary.

   Each commit introduces one coherent layer of authority and its tests. No cross-layer
   mega-commits.

## Scope Review Result

Cut from the core slice (beyond the cuts already made in `DESIGN_ACDC.md`):

- per-node failure directive map (`node_failures`) and `default_failure`;
- `RemediationObligationId`, `ManualResolutionSpecId`, and all reference-by-id maps;
- `FailureDirectiveSelected`, `RemediationObligationOpened`, `RemediationObligationClosed`,
  `ManualResolutionRequested`, and `RunTerminalResolved` as event families;
- spec-authored manual outcome catalogs, manual target specs, reason codes, and per-spec redaction
  policy;
- `PolicyCovered` typestate on node handles;
- generic retry policy, replan and continuation directives, trigger policy, correctness-claim APIs,
  resource-claim APIs, chain finality/reorg semantics;
- `RecoveringSideEffect`, `Replanning`, and `IrreversibleBlocked` public run modes;
- the post-core resource-claim milestone, enum, and test matrix. Resource claims get their own RFC
  after the core slice is proven.

The core v1 must prove only:

```text
confirmed forward side effects
  -> later non-retryable failure
  -> obligations derived from certified policy plus ledger evidence
  -> remediation-only side effects run in reverse confirmation order
  -> run resolves as Compensated, ManuallyResolved, or FailedWithoutAcdcClaim
```

## Architectural Decision

The v1 implementation is a certified saga implementation, not a durable bookkeeping layer.

All non-derivable semantic facts are persisted as append-only typed events. Runtime and store own
mode transitions as deterministic derivations from those facts. App, CLI, and REST surfaces display
or route these states; they are not semantic authority.

Layer ownership:

| Layer | V1 responsibility |
| --- | --- |
| `mfm-spec` | Hash-defining run-level saga policy, remediation node collection, manual evidence spec. |
| `mfm-certify` | Lowered-spec validation of saga structure. Reject ambiguous side-effecting workflows. |
| `mfm-events` | Ledger purpose on side-effect evidence, manual-resolution payload, extended terminal outcome. |
| `mfm-store` | Event admission, preconditions, derived obligation/run-mode projections, ledger purpose validation. |
| `mfm-runtime` | Failure-mode derivation, remediation frontier scheduling, sealed terminal resolution. |
| `mfm-replay` | Evidence-only validation of saga history. |
| `mfm-app` | Verified assembly and public status reporting. No remediation semantics. |
| storage crates | Durable persistence for new typed events/projections. No domain behavior. |

## Frozen V1 Decisions

These decisions are intentionally closed for the first implementation slice.

1. Saga policy is one run-level `SagaPolicySpec` living directly under `TypedExecutionSpec` as
   hash-defining certified data. There are no per-node directive overrides in v1.
2. Compensation linkage is per forward side-effect node: at most one remediation node per forward
   node, keyed by the forward node id. Obligation identity is derived from the forward node and its
   ledger key; there is no obligation ID namespace.
3. Remediation nodes live in a separate spec collection, structurally outside the forward graph.
   They reuse the existing `NodeSpec` shape as the element type. Forward reachability of a
   remediation node is unrepresentable, not certified away.
4. V1 reuses `ApplySideEffect` for remediation. Remedial ledgers carry persisted purpose
   `Remediation { forward_ledger_key }`. The obligation is derived from the forward ledger key;
   the purpose carries no second redundant field.
5. The stream gains exactly one new appendable semantic payload: `ManualResolutionRecorded`.
   Terminal resolution reuses the sealed lifecycle pattern: a sealed `ResolveSagaTerminal`
   framework node emits `RunCompleted` with an extended `RunCompletionOutcome`.
6. Directive selection, obligation open/close, run mode, and manual-block state are derived
   projections, never appendable events.
7. Manual resolution outcomes are a closed framework enum with exactly two variants:
   `ConfirmRemediated` and `FailWithoutAcdcClaim`. Manual resolution is run-scoped in v1;
   per-obligation manual targeting is deferred.
8. `RunCompletionOutcome` is rewritten to `Completed`, `Compensated`, `ManuallyResolved`, and
   `FailedWithoutAcdcClaim`. `Failed` and `Cancelled` are removed; cancellation is deferred until
   it is designed against remediation.
9. Remediation scheduling is strictly sequential in reverse forward-confirmation order, derived
   from stream order. Concurrent remediation is deferred.
10. Core v1 terminal claims are certified saga claims only. AC/DC-equivalence claims are deferred
    until verifier APIs exist.
11. Continuations, triggers, replan, retry policy, resource claims, irreversible-boundary APIs, and
    correctness classes are deferred.
12. Public status exposes semantic `RunMode`; coarse store `RunState` may remain internal.

## Core Semantic Rules

These rules are normative. The implementation must encode them as preconditions, projections, and
tests, not as comments.

### Saga policy engagement

Saga directives engage only when at least one forward side-effect ledger has crossed the durable
uncertainty boundary (invocation started). If a run fails non-retryably and no forward ledger
crossed the boundary, the run terminally resolves as `FailedWithoutAcdcClaim` with an empty
obligation set; public status shows that no external mutation occurred or could have occurred.

### Forward ledger eligibility

When a run under `CompensateCompleted` policy fails non-retryably, every forward side-effect ledger
is classified by its recorded phase:

| Forward ledger phase at failure | Classification | Consequence |
| --- | --- | --- |
| Intent / claimed / invocation prepared (before invocation started) | nothing owed | no obligation |
| Not-submitted proven, or failed with phase `BeforeInvocationStarted` / `AfterNotSubmittedProven` | nothing owed | no obligation |
| Confirmation observed | owed | remediation node scheduled |
| Invocation started / submission observed / submission unknown / receipt observed / ambiguous | unresolvable | run cannot claim `Compensated`; the `on_remediation_unresolved` directive applies |

Only confirmed forward ledgers are compensatable in v1. Any forward ledger past the uncertainty
boundary that is not confirmed and not proven-unsubmitted makes the run unresolvable by the
platform: it degrades through `on_remediation_unresolved` to `ManualBlocked` or
`FailedWithoutAcdcClaim`. `Compensated` is claimable only when the unresolvable set is empty and
every owed obligation closed with remedial confirmation evidence.

### Uniform unresolved rule

"Remediation unresolved" is one condition with one degradation path, covering all of:

- a forward ledger classified unresolvable at failure time;
- a remediation ledger that fails non-retryably;
- a remediation ledger that ends ambiguous.

In every case the certified `on_remediation_unresolved` directive applies: `ManualResolution`
blocks the run for typed operator evidence; `FailWithoutAcdcClaim` terminally resolves with the
unresolved obligations visible in public status. An ambiguous ledger can never satisfy obligation
closure.

### Coverage

Under `CompensateCompleted` policy, every forward `ApplySideEffect` node must have exactly one
linked remediation node. A spec with a compensating policy and an unlinked forward side-effect node
is invalid: it could otherwise terminate `Compensated` while a confirmed mutation stands
uncompensated. Builders enforce this at finalize; certification re-verifies it on lowered specs.

### Remediation ordering

Owed obligations are remediated strictly sequentially, in reverse order of the forward
confirmation events in the run stream. This order is deterministic, total (stream order breaks
ties between incomparable graph nodes), and respects dependencies, because a dependent node's
ledger confirms after its dependencies' ledgers.

### Remediation node input binding

A remediation node's binding tree may reference only:

- the linked forward node's typed output cell (which exists if and only if the forward ledger
  confirmed); and
- cells the linked forward node itself could reference (the forward node's ancestors).

Both are guaranteed materialized whenever the obligation is owed. Bindings to any other cell are
invalid: certification rejects them because the referenced cell may not exist in remediation mode.
This is how a compensation receives the forward effect's identifiers (operation id, transaction
hash, amounts) as typed input.

### Terminal resolution

Terminal resolution reuses the sealed framework lifecycle pattern. A sealed `ResolveSagaTerminal`
node — scheduled by runtime only when the derived run mode admits a terminal — emits `RunCompleted`
with the extended outcome. Historical validation extends the existing rule: each outcome variant is
accepted only when sealed lifecycle evidence exists and the derived projection agrees
(`Compensated` requires all owed obligations closed and no unresolvable ledgers; `ManuallyResolved`
and manual `FailedWithoutAcdcClaim` require admitted `ManualResolutionRecorded` evidence; policy
`FailedWithoutAcdcClaim` requires the certified policy to permit it). Successful forward completion
keeps the existing sealed `CompleteRun` path unchanged.

## V1 Data Model

The exact Rust names can change during implementation, but the semantic shape should not.

### Spec

```rust
pub struct TypedExecutionSpec {
    // existing hash-defining fields omitted
    pub saga: SagaPolicySpec,
    /// Remediation nodes, keyed by the forward side-effect node they compensate.
    /// Structurally outside the forward graph: the forward scheduler cannot see them.
    pub remediations: BTreeMap<NodeId, NodeSpec>,
}

pub enum SagaPolicySpec {
    /// Derived by lowering when the graph has no forward ApplySideEffect nodes.
    /// Failure is clean: no mutation was possible.
    NoSideEffects,
    /// Failure after mutations terminates with no compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
    /// Failure after mutations blocks for typed operator evidence.
    ManualResolution { manual: ManualResolutionEvidenceSpec },
    /// Failure after mutations compensates confirmed forward side effects.
    CompensateCompleted { on_remediation_unresolved: RemediationUnresolvedSpec },
}

pub enum RemediationUnresolvedSpec {
    ManualResolution { manual: ManualResolutionEvidenceSpec },
    FailWithoutAcdcClaim,
}

pub struct ManualResolutionEvidenceSpec {
    pub evidence_schema: SchemaId,
    pub operator_identity_ref_schema: SchemaId,
}
```

Notes:

- There is no `default_failure`, no per-node directive map, and no obligation map. The obligation
  set is derived: it is exactly the owed/unresolvable forward ledgers under the eligibility table.
- Manual specs are inlined where used. There is no manual-resolution ID namespace and nothing to
  dangle.
- `NoSideEffects` is produced by lowering, not authored. Certification rejects a spec whose policy
  variant disagrees with the presence of forward side-effect nodes, in both directions.
- All spec types are closed: no extension fields, deserialization denies unknown fields. Encoding
  deferred semantics (triggers, resources, continuations, correctness claims) is a parse failure,
  not a certification rule.

### Side-effect ledger purpose

```rust
pub enum SideEffectLedgerPurpose {
    Forward,
    Remediation { forward_ledger_key: SideEffectLedgerKey },
}
```

The purpose participates in ledger identity, event validation, projections, preconditions, and
replay. The obligation is derived from `forward_ledger_key`; there is no second linkage field to
keep consistent. At most one remediation ledger may exist per forward ledger key.

### Manual resolution

```rust
pub enum ManualResolutionOutcome {
    /// Operator certifies the unresolved obligations are remediated; run resolves ManuallyResolved.
    ConfirmRemediated,
    /// Operator abandons remediation; run resolves FailedWithoutAcdcClaim.
    FailWithoutAcdcClaim,
}
```

`ManualResolutionRecorded` carries the selected outcome, a typed operator identity reference
matching `operator_identity_ref_schema`, a typed evidence artifact matching `evidence_schema`, and
an optional redaction-safe note. The outcome enum is framework-closed: an operator cannot invent a
terminal meaning, and there is no spec-authored outcome catalog to certify. Manual resolution is
run-scoped in v1: one admitted record resolves the blocked run.

The reason a run is `ManualBlocked` is derivable (ambiguous forward ledger, failed remediation,
ambiguous remediation, or policy `ManualResolution`) and is exposed as a projection field, not
recorded in the spec or the event.

### Run modes

```rust
pub enum RunMode {
    Forward,
    Remediating,
    ManualBlocked,
    Completed,
    Compensated,
    ManuallyResolved,
    FailedWithoutAcdcClaim,
}
```

`RunMode` is a stream-derived projection. `Completed` means successful forward output.
`Compensated` means every owed obligation closed with remedial confirmation and no unresolvable
ledger exists. Core v1 never marks `Compensated` as an AC/DC-equivalent outcome.

### Events

New appendable payloads, in full:

```text
ManualResolutionRecorded            // operator evidence: the only genuinely new fact
RunCompleted { outcome }            // existing event, extended RunCompletionOutcome,
                                    // emitted by sealed CompleteRun or ResolveSagaTerminal
side-effect protocol events         // existing intent/claim/invocation/submission/receipt/
                                    // confirmation/ambiguity/failure, now carrying ledger purpose
```

Explicitly not events: directive selection, obligation opened, obligation closed, manual
requested, run mode changed. All are projections. What is never written cannot be forged and needs
no replay forgery check.

## Strictness Through Types

Preferred mechanisms, in priority order:

1. Structural unrepresentability in the spec data model: remediation nodes in a separate
   collection; ledger purpose with a single linkage field; inline manual specs; closed enums with
   deny-unknown-fields decoding.
2. Linked construction in the builder: the only way to author a compensated side effect is one
   call that creates and links both nodes:

   ```rust
   let (forward, remediation) =
       program.side_effect_with_compensation(forward_state, compensation_state);
   ```

   "Obligation without target", "obligation targeting a non-side-effect node", "remediation node
   in the forward graph", and "compensating policy with zero links" are unconstructible, not
   checked.
3. Branded handles distinguishing forward side-effect nodes from remediation nodes. This is a
   local, per-handle property and earns its keep.
4. Finalize-time validation with typed errors for whole-graph properties: coverage (every forward
   side-effect node linked under a compensating policy) and remediation binding scope. Do not
   encode graph-global properties as handle typestate; `PolicyCovered`-style states cannot honestly
   express them and add ceremony without unrepresentability.

Lowered specs, persisted bytes, registry inputs, and migration outputs remain hostile.
Certification re-verifies the same structural facts, but because the lowered shape carries most
invariants structurally, certification reduces to decoding, hash checks, and the graph rules below.

## Breaking Change Posture

Breaking changes are allowed and expected.

- Bump persisted spec/event schemas; `SagaPolicySpec` and `remediations` are hash-defining.
- Rewrite `RunCompletionOutcome` to the four-variant terminal vocabulary. Remove `Failed` and
  `Cancelled`. Cancellation returns only with designed remediation semantics.
- Replace public absent/started/completed status with semantic `RunMode`.
- Reject old side-effecting specs that lack certified saga policy.
- Remove or rewrite APIs that allow raw construction of semantically invalid specs, nodes, events,
  or terminal states.
- No best-effort compatibility shims. If old persisted runs need support later, that is an explicit
  migration tool or read-only archival mode.

## Certification Rules

Builders make most invalid saga structure unconstructible (see above). Certification verifies
lowered specs as the hostile-bytes authority boundary:

- the `SagaPolicySpec` variant agrees with the presence of forward `ApplySideEffect` nodes
  (`NoSideEffects` if and only if there are none);
- every `remediations` key references an existing forward `ApplySideEffect` node;
- under `CompensateCompleted`, every forward `ApplySideEffect` node has exactly one remediation
  entry;
- every remediation node is side-effect-grade (`ApplySideEffect` descriptor with a matching
  side-effect contract digest);
- every remediation node's bindings reference only the linked forward node's output cell and the
  forward node's ancestor cells;
- manual specs carry resolvable evidence and operator identity schemas.

Rules that no longer exist because the shape carries them: forward reachability of remediation
nodes (separate collection), dangling obligation/manual ids (no id namespaces), zero-obligation
compensate directives (run-level policy plus coverage), unsupported extension fields (closed types,
deny-unknown-fields).

## Runtime Semantics

V1 runtime behavior proves one complete saga path with two forward side effects.

Happy remediation path:

1. Run executes the forward graph; two forward `ApplySideEffect` ledgers reach confirmation.
2. A later node fails non-retryably.
3. Runtime derives the directive from certified policy plus recorded facts; no directive event is
   appended. The run mode projection becomes `Remediating`.
4. Runtime classifies forward ledgers per the eligibility table; both are owed.
5. Runtime schedules the linked remediation nodes strictly sequentially in reverse confirmation
   order. Remedial ledgers run the existing side-effect protocol with purpose
   `Remediation { forward_ledger_key }`.
6. Each obligation closes when its remedial confirmation event is recorded (derived, not appended).
7. Runtime schedules sealed `ResolveSagaTerminal`; `RunCompleted { Compensated }` is committed.

Manual path:

1. Any unresolved condition under the uniform unresolved rule arises, or policy is
   `ManualResolution`.
2. The run mode projection becomes `ManualBlocked` with a derived block reason. Nothing is
   appended.
3. An operator submits typed evidence; store admits `ManualResolutionRecorded` only while the run
   is derivably `ManualBlocked` and the evidence matches the certified schemas.
4. Runtime schedules sealed `ResolveSagaTerminal`; the outcome follows the recorded
   `ManualResolutionOutcome`.

Fail-without-claim path:

1. Failure occurs; certified policy (or the unresolved directive) is `FailWithoutAcdcClaim`, or no
   forward ledger crossed the uncertainty boundary.
2. Runtime schedules sealed `ResolveSagaTerminal`;
   `RunCompleted { FailedWithoutAcdcClaim }` is committed.
3. Public status shows the obligation set, including unresolved obligations and the
   nothing-crossed-the-boundary case, so the absence of a claim is explicit.

The forward frontier scheduler never selects a remediation node: remediation nodes are not in the
forward graph. The remediation frontier scheduler runs only in derived `Remediating` mode and never
selects forward nodes. A non-retryable failure of a remediation node consults
`on_remediation_unresolved`, never a forward policy.

## Store Semantics

Store remains append-only authority. Projections are rebuildable caches.

V1 store work:

- persist ledger purpose on side-effect events; purpose participates in ledger identity;
- admit at most one remediation ledger per forward ledger key;
- admit a remediation ledger only when its forward ledger is confirmed, the certified policy is
  compensating, and a qualifying non-retryable failure event exists in the stream;
- admit `ManualResolutionRecorded` only while the derived run mode is `ManualBlocked`, once per
  run, with schema-valid payloads;
- admit `RunCompleted` only with sealed lifecycle evidence and a derived projection that agrees
  with the claimed outcome;
- derive obligation and run-mode projections from the stream; projection rebuild parity is tested.

The existing coarse `RunState` may remain for commit preconditions; it is not the public status
model.

## Replay Semantics

Replay must not call live transports.

V1 replay work:

- index side-effect evidence by ledger purpose;
- index `ManualResolutionRecorded` and terminal resolution evidence;
- recompute the derived obligation classification and run mode from certified spec plus indexed
  facts, and verify the recorded terminal outcome agrees:
  - `Compensated` requires every owed obligation closed by remedial confirmation and an empty
    unresolvable set;
  - `ManuallyResolved` and manual `FailedWithoutAcdcClaim` require admitted, schema-valid operator
    evidence;
  - policy `FailedWithoutAcdcClaim` requires the certified policy to permit it;
- verify each remediation ledger links to a confirmed forward ledger of the same run;
- reject terminal outcomes the recomputation does not support.

Because obligation state is derived rather than recorded, replay has no forged-control-event class
to detect: it recomputes and compares. Domain truth and AC/DC-equivalence proof remain outside core
v1.

## Public Surfaces

App/CLI/API status should expose:

- `RunMode`;
- the certified saga policy variant;
- the derived obligation set: per forward ledger, its classification (nothing owed, owed,
  unresolvable) and remediation ledger state;
- linked forward and remediation ledger keys;
- the derived manual-block reason and required evidence schemas when `ManualBlocked`;
- terminal resolution and which claim it carries.

This is a breaking public status change. It is allowed.

## API And LOC Budget

Required public API additions:

- `side_effect_with_compensation` (or equivalent linked-construction surface) and branded
  forward/remediation handles;
- closed `SagaPolicySpec`, `RemediationUnresolvedSpec`, `ManualResolutionOutcome`, and `RunMode`
  enums;
- bounded manual-resolution evidence types;
- public status fields for run mode and the derived obligation set.

Required internal or persisted additions:

- hash-defining `SagaPolicySpec` and `remediations` collection;
- `SideEffectLedgerPurpose` on ledger evidence;
- `ManualResolutionRecorded` payload and extended `RunCompletionOutcome`;
- sealed `ResolveSagaTerminal` lifecycle node;
- derived obligation/run-mode projections and their preconditions;
- replay recomputation for saga history.

Avoid in v1:

- new effect class hierarchy;
- per-node directive maps and obligation/manual ID namespaces;
- control events for derivable decisions;
- generic remediation handler traits, per-state compensation methods, dynamic policy callbacks;
- exposing store projection internals as authoring API;
- compatibility adapters for old run-status semantics;
- framework implementations of commutativity, escrow, predicate isolation, resource claims, or
  reorg semantics.

## Implementation Milestones

### Milestone 0: contract PR

Purpose: land type skeletons and compile-time contracts without runtime behavior.

Scope:

- spec types: `SagaPolicySpec`, `remediations` collection, manual evidence spec;
- `SideEffectLedgerPurpose` and `ManualResolutionOutcome` types;
- extended `RunCompletionOutcome` and `RunMode` types;
- rustdoc explaining saga-only external semantics and the fact/derivation split.

Acceptance:

- `cargo check --workspace`;
- canonical JSON/spec hash tests for the new hash-defining fields;
- no runtime behavior changes yet.

### Milestone 1: builders and certification

Purpose: make invalid saga structure unconstructible, and re-verify lowered specs.

Scope:

- `side_effect_with_compensation` linked construction and branded handles;
- finalize-time coverage and binding-scope validation with typed errors;
- certification rules from this RFC;
- negative tests: policy/graph disagreement, unlinked forward node under compensating policy,
  remediation key referencing a missing or non-side-effect node, out-of-scope remediation
  bindings, manual spec without schemas.

Acceptance:

- `cargo test -p mfm-certify`;
- program builder tests, including compile-fail fixtures for handle misuse.

### Milestone 2: events and store projections

Purpose: persist remediation authority and derive saga state.

Scope:

- ledger purpose in event payloads and ledger identity;
- `ManualResolutionRecorded` payload and admission preconditions;
- extended `RunCompleted` admission preconditions;
- derived obligation and run-mode projections (in-memory and Postgres);
- append/rebuild projection parity tests.

Acceptance:

- `cargo test -p mfm-events`;
- `cargo test -p mfm-store`;
- the focused Postgres storage package tests.

### Milestone 3: runtime remediation

Purpose: prove runtime-owned saga transition.

Scope:

- derive `Remediating` from non-retryable failure after confirmed side effects;
- eligibility classification;
- sequential reverse-confirmation-order remediation scheduling;
- uniform unresolved handling into `ManualBlocked` or terminal;
- sealed `ResolveSagaTerminal` lifecycle node.

Acceptance:

- two confirmed side effects, later failure, both compensated in reverse confirmation order;
- crash/resume at every boundary: after failure, after first remedial submission, after first
  remedial confirmation, before terminal resolution — no duplicated mutation;
- forward scheduler provably cannot select remediation nodes; remediation scheduler provably
  cannot select forward nodes;
- ambiguous forward ledger at failure time degrades per policy, never `Compensated`.

### Milestone 4: replay

Purpose: make the compensated run independently verifiable.

Scope:

- replay indexes for ledger purpose, operator evidence, and terminal evidence;
- derived-state recomputation and terminal-agreement verification;
- rejection tests for unsupported terminal claims.

Acceptance:

- compensated run verifies;
- `Compensated` without a remedial confirmation rejects;
- remediation ledger linked to an unconfirmed or foreign forward ledger rejects;
- `ManuallyResolved` without schema-valid operator evidence rejects.

### Milestone 5: public status

Purpose: expose semantics to users and operators.

Scope:

- app status maps stream/projections to `RunMode` and the derived obligation set;
- CLI/API output reports obligations, manual-block reason, and required evidence schemas;
- docs update for the public status contract.

Acceptance:

- app tests for `Forward`, `Remediating`, `ManualBlocked`, `Compensated`, `ManuallyResolved`,
  `FailedWithoutAcdcClaim`;
- CLI JSON contract tests if CLI output changes.

## Reviewable Commit Plan

Each commit compiles and includes the tests that make sense for that layer. Commit subjects stay
narrow and lower-case.

1. `spec: add certified saga policy types`

   `SagaPolicySpec`, `remediations` collection, manual evidence spec, ledger purpose, manual
   outcome, run mode, extended run completion outcome. Hash-defining where applicable. Pure type
   skeleton, explicitly marked.

2. `program: add linked compensation authoring`

   `side_effect_with_compensation`, branded forward/remediation handles, finalize-time coverage
   and binding-scope checks with typed errors and compile-fail fixtures.

3. `certify: verify saga structure of lowered specs`

   The certification rules of this RFC with negative tests.

4. `events: carry ledger purpose and manual resolution`

   Ledger purpose on side-effect payloads and ledger identity; `ManualResolutionRecorded`;
   `RunCompletionOutcome` rewrite.

5. `store: derive saga projections and preconditions`

   Derived obligation/run-mode projections; admission preconditions for remedial ledgers, manual
   evidence, and terminal outcomes; rebuild parity tests.

6. `postgres: persist saga events and projections`

   Storage-only: no runtime or domain decisions.

7. `runtime: derive and schedule remediation`

   Failure classification, eligibility table, sequential reverse-confirmation scheduling, and the
   proof that the two frontiers cannot select each other's nodes.

8. `runtime: resolve saga terminals`

   Sealed `ResolveSagaTerminal`, uniform unresolved handling, `ManualBlocked` flow, terminal
   commits for all three saga outcomes.

9. `replay: verify saga evidence`

   Recompute-and-compare verification with the rejection matrix.

10. `app: expose semantic run mode`

    Replace user-facing absent/started/completed status with `RunMode` and obligation details.
    Update app/CLI/API docs and contract tests in the same commit.

Do not merge commits that add a semantic shape without its corresponding rejection tests unless the
commit is a pure type skeleton explicitly marked as such. Do not combine runtime remediation,
replay verification, and public status in one commit.

## Test Matrix

The implementation is not credible until these tests exist.

| Area | Required tests |
| --- | --- |
| Builders | Unlinked forward side-effect node under compensating policy fails finalize; remediation handle unusable as forward node (compile-fail); out-of-scope remediation binding fails finalize. |
| Certification | Policy/graph variant disagreement rejected; remediation key to missing or non-side-effect node rejected; coverage gap rejected; non-side-effect-grade remediation node rejected; manual spec without schemas rejected. |
| Store | Remediation ledger requires confirmed forward ledger plus qualifying failure; second remediation ledger for same forward key rejected; manual evidence admitted only while derivably `ManualBlocked`, once; terminal outcome disagreeing with derived projection rejected; projections rebuild from stream. |
| Runtime | Two confirmed effects then later failure remediate in reverse confirmation order; saga engages only past the uncertainty boundary (clean failure otherwise); ambiguous forward ledger degrades per policy; remediation failure/ambiguity follows `on_remediation_unresolved`; crash/resume at every remediation boundary; frontier separation proven both directions. |
| Replay | Compensated run verifies; `Compensated` with a missing remedial confirmation rejects; remediation ledger with wrong forward linkage rejects; manual evidence schema mismatch rejects; terminal claim unsupported by recomputation rejects. |
| Public status | Status distinguishes all seven run modes and shows per-ledger obligation classification, including unresolved obligations under `FailedWithoutAcdcClaim`. |

## First End-to-End Slice

Build one proof workflow with two forward mutations so reverse ordering is actually exercised:

```text
Pure setup
  -> ApplySideEffect forward mutation A
  -> ApplySideEffect forward mutation B (depends on A)
  -> Pure or ReadExternal state that fails non-retryably
  -> remediation of B, then remediation of A (reverse confirmation order)
  -> terminal Compensated run mode
```

Required proof:

- both forward side effects reach confirmation;
- later failure does not produce ordinary completion and appends no control events;
- remediation ledgers carry purpose `Remediation { forward_ledger_key }` and run B-then-A;
- crash/resume duplicates no forward or remedial mutation;
- replay verifies the run without live IO by recomputing derived state;
- public status reports `Compensated` with both obligations closed.

## Risks

- If saga policy is not hash-defining certified spec data, resume/replay authority becomes
  ambiguous.
- If ledger purpose is not persisted and part of ledger identity, remediation is only scheduling
  convention.
- If derived projections drift from the derivation rules in this RFC, replay and store can
  disagree; the recompute-and-compare replay tests and rebuild parity tests are the guard.
- If manual resolution grows spec-authored outcomes, it becomes a generic escape hatch again.
- If the eligibility table is weakened (e.g., compensating from receipt-only evidence),
  `Compensated` stops being a sound claim.
- If cancellation is reintroduced without remediation-aware design, it reopens the forged-terminal
  hole the current history validation closed.
- Run-level-only policy may prove too coarse for real workflows; per-node overrides are the known
  next step and must come back as certified spec data, not callbacks.

## Deferred Work

- Per-node failure directive overrides and retry policy.
- Per-obligation manual resolution targeting and manual outcome catalogs.
- Run cancellation semantics under remediation.
- Concurrent remediation scheduling.
- Resource claims and concurrency correctness (separate RFC after the core slice is proven).
- `ApplyCompensation` effect class.
- Generic commutativity, escrow, predicate isolation, and phantom-proof framework semantics.
- Chain-specific reorg/finality verifiers and triggers.
- Continuation child-run execution beyond typed parent/child linkage.
- AC/DC-equivalence claims and domain verifier APIs.

## Definition Of Done For V1

V1 is done when MFM can run, persist, resume, replay, and inspect the two-mutation end-to-end slice
above, and the test matrix proves that compensation is certified runtime/store/replay behavior —
derived from certified policy plus append-only facts — rather than ordinary user-state bookkeeping.
