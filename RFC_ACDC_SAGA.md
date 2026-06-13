# RFC: certified saga vertical slice for MFM

Status: ready for engineering planning. Revised after two independent architecture reviews; this
revision supersedes the v1 scope in `DESIGN_ACDC.md` where they disagree. The second review pass
fixed admission-authority placement, derivation timing (engagement fence and classification
quiescence), ambiguity engagement, terminal-outcome justification, lane lifecycle, and the
hostile-bytes disjointness rule.

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
5. split admission authority: stream-derivable store preconditions plus runtime-constructed
   spec-aware preconditions, with replay as the policy-agreement backstop;
6. replay verification without live IO;
7. public status that reports semantic run modes and per-ledger obligation state;
8. after the core slice: a minimal cross-run resource-claim surface — `Exclusive` lanes,
   `ExactTouchedSet` evidence, and `ManualOnly` degradation — so concurrent isolated runs that
   mutate the same external resource are sequenced or honestly unclaimed.

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
- Do not implement generic commutativity, escrow, predicate isolation, or chain reorg semantics in
  the kernel.
- Do not add resource derivation enums, lane fairness/queueing policy, or cross-run deadlock
  detection in the first resource-claim slice.
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
   structurally outside the forward graph cannot be forward-scheduled by any builder-authored
   spec; certification closes the remaining hostile-bytes surface by requiring the forward and
   remediation node-id sets to be disjoint. Certification still verifies lowered specs, but every
   invariant that the data shape can carry is one certification rule and one forgery surface
   deleted.

2. Record facts, derive decisions.

   The append-only stream records only non-derivable facts: attempt lifecycle, ledger IO evidence,
   operator evidence, and sealed terminal resolution. Everything deterministic from certified spec
   plus those facts — selected directive, obligation state, run mode — is a rebuildable projection.
   A decision that is never written cannot be forged and never needs a replay forgery check.
   A derivation is only as well-defined as its timing rules: the engagement fence and
   classification quiescence rules below are part of this constraint, not implementation detail.

3. Keep the public API and LOC growth narrow.

   V1 adds only: run-level saga policy, a linked-compensation authoring surface, ledger purpose,
   one operator evidence event, one sealed terminal lifecycle node, semantic run mode, and the
   projections behind public status. No generic callback systems, no broad new traits, no duplicate
   side-effect machinery, no speculative correctness classes. The post-core resource-claim
   milestone is similarly bounded: one mandatory spec field, a derived lane projection, and
   evidence checks on existing events.

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
  chain finality/reorg semantics;
- `RecoveringSideEffect`, `Replanning`, and `IrreversibleBlocked` public run modes;
Resource claims stay out of the core slice but return as a bounded post-core milestone in this RFC
(see Cross-Run Resource Claims): the same `Exclusive` / `ExactTouchedSet` / `ManualOnly` vocabulary
as before, redesigned under this RFC's principles — no derivation enums, no policy maps, no new
event families, no new run modes.

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
| `mfm-store` | Event admission, stream-derivable saga preconditions, the forward-progress fence, ledger purpose validation, derived projections as pure functions of certified policy plus stream. |
| `mfm-runtime` | Failure-mode derivation, quiescence driving, spec-aware admission preconditions, remediation frontier scheduling, sealed terminal resolution. |
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
   it is designed against remediation. Non-`Completed` variants carry no payload; the obligation
   set, block reason, and justification are derivable.
9. Remediation scheduling is strictly sequential in reverse forward-confirmation order, derived
   from stream order. Concurrent remediation is deferred.
10. Core v1 terminal claims are certified saga claims only. AC/DC-equivalence claims are deferred
    until verifier APIs exist.
11. Continuations, triggers, replan, retry policy, resource claims, irreversible-boundary APIs, and
    correctness classes are deferred.
12. Public status exposes semantic `RunMode`; coarse store `RunState` may remain internal.
13. Resource claims (post-core, Milestone 6) are a mandatory closed field on every
    `ApplySideEffect` node spec: `Exclusive`, `ExactTouchedSet`, or `ManualOnly`. There is no
    derivation enum; adapters derive keys and touched sets as typed evidence under certified
    schemas.
14. Exclusive lanes are derived cross-run projections: acquisition is the resource key recorded at
    invocation-prepared; release is the holding ledger's terminal evidence or the holding run's
    sealed terminal resolution. A held lane blocks every other ledger, same-run or cross-run. No
    lane-control events, no fairness guarantee, no new run mode.
15. Remediation ledgers acquire lanes under the same rules as forward ledgers.
16. Saga admission authority is split. The store independently enforces every stream-derivable
    precondition (ledger phases, the forward-progress fence, quiescence at terminal and manual
    admission, at-most-one remediation ledger per forward key, at-most-one manual record per run)
    and stays spec-blind; the runtime constructs the spec-aware preconditions (policy variant,
    run-mode agreement, evidence schemas) from `CertifiedRuntimeSpec`; replay re-verifies all
    spec-aware conditions as the hostile backstop.
17. Saga engagement conditions are closed: a recorded non-retryable failure, or a forward ledger
    entering ambiguity. The first engaging event in stream order anchors engagement. Obligation
    classification is defined over the full stream at quiescence, never pinned at the engaging
    event's position.

## Core Semantic Rules

These rules are normative. The implementation must encode them as preconditions, projections, and
tests, not as comments.

### Saga policy engagement

A run becomes saga-engaged at the first engaging event in stream order. Exactly two conditions
engage: a recorded non-retryable failure (`StateAttemptFailed` / `SideEffectFailed` with
`retryable = false`) and a forward side-effect ledger entering ambiguity — with or without any
other failure. Ambiguity must engage on its own: the forward scheduler blocks the whole frontier
on an ambiguous ledger, so the downstream failure that would otherwise engage may never arrive,
and without engagement the run would have no path to `ManualBlocked`, manual resolution, or any
terminal. Later engaging events change nothing; the first one anchors engagement.

Saga directives apply only when at least one forward side-effect ledger has crossed the durable
uncertainty boundary (invocation started). If the run engages and no forward ledger crossed the
boundary, the run terminally resolves as `FailedWithoutAcdcClaim` with an empty obligation set;
public status shows that no external mutation occurred or could have occurred. This clean-failure
resolution is legal under every policy variant.

### Engagement fence and classification quiescence

The deleted control events had a hidden job: pinning when classification happens. These three
rules replace it. All are stream-derivable and all are normative.

1. Fence. After the first engaging event, the store admits no new forward boundary crossings: no
   new forward intent, claim, invocation-prepared, or invocation-started events. Recovery and
   terminal evidence for forward ledgers already past the boundary — submission results,
   not-submitted proof, receipt, confirmation, ambiguity, failure — remains admissible. The fence
   derives from recorded `retryable` flags and ambiguity events alone; the store enforces it with
   no spec access.
2. Quiescence. Before the owed and unresolvable sets are final, runtime drives every
   past-boundary, non-quiescent forward ledger to a quiescent phase: confirmation observed,
   not-submitted proven (or its legal failure), or ambiguous. An in-flight forward attempt at
   engagement time is not classified unresolvable while it can still legally progress; it is
   classified by the quiescent phase it reaches. Existing recovery transitions (for example
   submission-unknown to not-submitted-proven) run to completion under the fence.
3. Full-stream classification. Obligation classification and run mode are defined over the entire
   current stream, never pinned at the engaging event's position. `ResolveSagaTerminal`
   scheduling, `ManualResolutionRecorded` admission, and `RunCompleted` admission all require
   quiescence. With the fence, the derived sets are monotone: two readers of the same stream
   derive the same obligation state, and a longer prefix only refines non-quiescent ledgers
   toward quiescent phases.

Confirmation evidence committed after the engaging event — a racing claim-takeover worker, an
in-flight attempt finishing — therefore classifies its ledger as owed, not unresolvable, and the
obligation joins the reverse-confirmation order at its recorded stream position.

### Forward ledger eligibility

When a run under `CompensateCompleted` policy engages, every forward side-effect ledger is
classified by its recorded phase at quiescence:

| Forward ledger phase at quiescence | Classification | Consequence |
| --- | --- | --- |
| Intent / claimed / invocation prepared (never crossed the boundary; frozen by the fence) | nothing owed | no obligation |
| Not-submitted proven, or failed with phase `BeforeInvocationStarted` / `AfterNotSubmittedProven` | nothing owed | no obligation |
| Confirmation observed | owed | remediation node scheduled |
| Ambiguous | unresolvable | run cannot claim `Compensated`; the `on_remediation_unresolved` directive applies |

Invocation started, submission observed, submission unknown, and receipt observed are not
quiescent phases: runtime drives them to a row above before classification is final. An adapter
that cannot decide must record ambiguity as evidence, not hang.

Only confirmed forward ledgers are compensatable in v1. Any past-boundary forward ledger that is
ambiguous at quiescence makes the run unresolvable by the platform: it degrades through
`on_remediation_unresolved` to `ManualBlocked` or `FailedWithoutAcdcClaim`. `Compensated` is
claimable only when the owed set is non-empty, every owed obligation closed with remedial
confirmation evidence, and the unresolvable set is empty. If the boundary was crossed but
quiescence leaves both sets empty — every past-boundary ledger proved unsubmitted or failed
legally — the run resolves `FailedWithoutAcdcClaim` with an empty obligation set. `Compensated`
always means at least one remediation actually ran; it is never claimable vacuously.

### Uniform unresolved rule

"Remediation unresolved" is one condition with one degradation path, covering all of:

- a forward ledger ambiguous at quiescence, with or without any other failure;
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

Authoring guidance: an inherently irreversible forward effect (a sent notification, dispensed
funds) has no honest compensation. Do not link a no-op remediation node to satisfy coverage; that
quietly degrades what `Compensated` means. Until per-node directives exist, workflows containing
such effects should choose run-level `ManualResolution` or `FailWithoutAcdcClaim`.

### Remediation ordering

Owed obligations are remediated strictly sequentially, in reverse order of the forward
confirmation events in the run stream. This order is deterministic, total (stream order breaks
ties between incomparable graph nodes), and respects dependencies, because a dependent node's
ledger confirms after its dependencies' ledgers.

Remediation stops at the first unresolved condition. When a remediation ledger fails non-retryably
or ends ambiguous, `on_remediation_unresolved` applies immediately; remaining owed obligations are
not attempted, even though their remediations might have succeeded. This is deliberate and
conservative. Never-attempted owed obligations stay visible in public status as owed and
unremediated under the eventual terminal.

### Remediation node input binding

A remediation node's binding tree may reference only:

- the linked forward node's typed output cell (which exists once the forward ledger confirmed
  and forward completion work finished; see the materialization rule under Runtime Semantics);
  and
- cells the linked forward node itself could reference (the forward node's ancestors).

Both are guaranteed materialized before any remediation node is scheduled. Bindings to any other cell are
invalid: certification rejects them because the referenced cell may not exist in remediation mode.
This is how a compensation receives the forward effect's identifiers (operation id, transaction
hash, amounts) as typed input.

### Terminal resolution

Terminal resolution reuses the sealed framework lifecycle pattern. Lowering inserts exactly one
sealed `ResolveSagaTerminal` framework node into every spec, alongside the existing exactly-one
`CompleteRun` node; certification verifies both counts. Runtime schedules `ResolveSagaTerminal`
only when the derived run mode admits a terminal and quiescence holds; it emits `RunCompleted`
with the extended outcome. Non-`Completed` outcome variants carry no payload: the obligation set,
block reason, and justification are all derivable.

Historical validation extends the existing rule: each outcome variant is accepted only when sealed
lifecycle evidence exists and the derived projection agrees:

- `Completed` requires the existing sealed `CompleteRun` evidence, unchanged;
- `Compensated` requires a non-empty owed set with every owed obligation closed by remedial
  confirmation, an empty unresolvable set, and quiescence;
- `ManuallyResolved` requires an admitted `ManualResolutionRecorded` with outcome
  `ConfirmRemediated`;
- `FailedWithoutAcdcClaim` is justified by exactly one of four conditions, and validation must
  accept all four: (1) the certified policy is `FailWithoutAcdcClaim`; (2) the policy is
  `CompensateCompleted` with `on_remediation_unresolved: FailWithoutAcdcClaim` and the unresolved
  set is non-empty; (3) the run engaged with no forward ledger past the uncertainty boundary, or
  quiescence left both the owed and unresolvable sets empty — legal under every policy variant;
  (4) an admitted `ManualResolutionRecorded` carries outcome `FailWithoutAcdcClaim`.

Successful forward completion keeps the existing sealed `CompleteRun` path unchanged.

### Cross-run concurrency

Runs are isolated append-only histories. Core v1 makes no cross-run claim: `Compensated` is a
single-run claim and says nothing about concurrent runs that mutated the same external resources.
The bounded mechanism that adds cross-run sequencing is specified in Cross-Run Resource Claims and
lands as Milestone 6; until it lands, conflicting concurrent runs are the domain's responsibility
and no platform concurrency claim exists.

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
- manual specs carry resolvable evidence and operator identity schemas;
- the forward `nodes` collection and the `remediations` collection have disjoint node-id sets;
  hostile bytes must not be able to place one node in both graphs;
- `remediations` is empty unless the policy is `CompensateCompleted`; dead remediation data under
  a non-compensating policy is rejected;
- exactly one sealed `ResolveSagaTerminal` framework node exists, under the same exactly-one rule
  as `CompleteRun`.

Rules that no longer exist because the shape carries them: dangling obligation/manual ids (no id
namespaces), zero-obligation compensate directives (run-level policy plus coverage), unsupported
extension fields (closed types, deny-unknown-fields). Forward reachability of remediation nodes is
carried by the separate collection for builder-authored specs; the disjointness rule above closes
the hostile-bytes remainder.

## Runtime Semantics

V1 runtime behavior proves one complete saga path with two forward side effects.

Happy remediation path:

1. Run executes the forward graph; two forward `ApplySideEffect` ledgers reach confirmation.
2. A later node fails non-retryably. The fence engages; no new forward boundary crossing is
   admissible.
3. Runtime derives the directive from certified policy plus recorded facts; no directive event is
   appended. The run mode projection becomes `Remediating`.
4. Runtime drives any past-boundary, non-quiescent forward ledger to a quiescent phase, completes
   forward output-cell materialization for confirmed ledgers, then classifies forward ledgers per
   the eligibility table; both are owed.
5. Runtime schedules the linked remediation nodes strictly sequentially in reverse confirmation
   order. Remedial ledgers run the existing side-effect protocol with purpose
   `Remediation { forward_ledger_key }`.
6. Each obligation closes when its remedial confirmation event is recorded (derived, not appended).
7. Runtime schedules sealed `ResolveSagaTerminal`; `RunCompleted { Compensated }` is committed.

Manual path:

1. Any unresolved condition under the uniform unresolved rule arises — including a forward
   ledger that ends ambiguous with no other failure — or policy is `ManualResolution`.
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

Two forward tasks survive engagement, and the frontier-separation rule does not forbid them
because they are completion work owed to already-recorded evidence, not frontier scheduling:

1. Quiescence driving: recovery and evidence transitions for forward ledgers already past the
   boundary, until each reaches a quiescent phase.
2. Output-cell materialization: confirmation evidence and the forward node's terminal output cell
   commit separately, so a crash can leave a ledger confirmed without its output cell. On resume
   or on entering `Remediating`, runtime first completes terminal output materialization for every
   confirmed forward ledger missing its output cell. Remediation bindings depend on it.

## Store Semantics

Store remains append-only authority and remains spec-blind: it never opens spec or certificate
artifacts. Projections are rebuildable caches. Saga admission authority is split three ways.

Store-enforced independently (stream-derivable, no spec access):

- persist ledger purpose on side-effect events; purpose participates in ledger identity;
- the forward-progress fence: after the first engaging event, reject new forward intent, claim,
  invocation-prepared, and invocation-started events; recovery and terminal evidence for
  past-boundary ledgers stays admissible;
- admit at most one remediation ledger per forward ledger key;
- admit a remediation ledger only when its forward ledger is confirmed and an engaging event
  exists in the stream;
- admit at most one `ManualResolutionRecorded` per run, and only at quiescence;
- admit `RunCompleted` only for a started, non-terminal run, and only at quiescence (no
  past-boundary forward ledger in a non-quiescent phase).

Runtime-constructed spec-aware preconditions, built from `CertifiedRuntimeSpec`, validated by the
store as ordinary preconditions, re-verified by replay:

- the certified policy is compensating before any remediation ledger opens;
- the derived run mode is `ManualBlocked`, and the manual payload matches the certified evidence
  and operator-identity schemas, before `ManualResolutionRecorded` is admitted;
- sealed lifecycle evidence exists and the claimed terminal outcome agrees with the derived
  projection before `RunCompleted` is admitted.

A buggy or hostile runtime can at worst append events the spec-aware rules forbid; replay's
recompute-and-compare rejects the run, and the store's independent checks bound the damage. The
store is not the policy authority and does not pretend to be.

Derived obligation and run-mode projections are pure functions of certified saga policy plus the
stream. The derivation code lives in `mfm-store` so store callers, runtime, and replay share one
implementation; the policy input is supplied by callers that hold certified authority. Projection
rebuild parity is tested.

The existing coarse `RunState` may remain for commit preconditions; it is not the public status
model.

## Replay Semantics

Replay must not call live transports.

V1 replay work:

- index side-effect evidence by ledger purpose;
- index `ManualResolutionRecorded` and terminal resolution evidence;
- recompute the derived obligation classification and run mode from certified spec plus indexed
  facts over the full stream, and verify the recorded terminal outcome agrees:
  - `Compensated` requires a non-empty owed set, every owed obligation closed by remedial
    confirmation, an empty unresolvable set, and quiescence at the terminal;
  - `ManuallyResolved` requires admitted, schema-valid operator evidence with outcome
    `ConfirmRemediated`;
  - `FailedWithoutAcdcClaim` requires one of its four justifications (policy, unresolved
    directive, clean failure or empty quiescent sets, manual outcome);
- verify the fence held: no forward boundary crossing follows the first engaging event;
- verify quiescence at terminal resolution: every past-boundary forward ledger reached a quiescent
  phase before `RunCompleted`;
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
  unresolvable) and remediation ledger state, including owed obligations never attempted because
  remediation stopped at an earlier unresolved condition;
- linked forward and remediation ledger keys;
- the derived manual-block reason and required evidence schemas when `ManualBlocked`;
- terminal resolution and which claim it carries;
- once Milestone 6 lands: per-ledger declared resource claim, the recorded key or touched set, and
  blocked-on-lane detail (namespace, key, holding run) when a node is waiting.

This is a breaking public status change. It is allowed.

## Cross-Run Resource Claims

Post-core scope: this section lands as Milestone 6, after the core saga slice is proven. It is the
minimal API for concurrent and parallel isolated runs, in the established vocabulary — `Exclusive`,
`ExactTouchedSet`, `ManualOnly` — redesigned under this RFC's principles: no derivation enums, no
policy maps, no new event families, no new run modes.

### Spec

Every `ApplySideEffect` node spec — forward and remediation — gains one mandatory hash-defining
field:

```rust
pub enum ResourceClaimSpec {
    /// Concrete key derivable from typed intent before mutation;
    /// the store serializes conflicting lanes across runs.
    Exclusive { namespace: ResourceNamespace, key_schema: SchemaId },
    /// The affected key set is knowable only after execution;
    /// the attempt records the actual touched set as typed evidence.
    ExactTouchedSet { namespace: ResourceNamespace, evidence_schema: SchemaId },
    /// MFM derives and verifies nothing; no platform concurrency claim.
    ManualOnly,
}
```

Because the field is mandatory, the old "every mutating node must expose a claim class"
certification rule is carried by the type; `ManualOnly` is the explicit no-claim choice. Builders
take the claim at node construction (`side_effect_with_compensation` gains the parameters), so
coverage is structural. `ResourceNamespace` follows capability naming
(`mfm.evm.account_nonce`); the canonical first lane is chain id plus sender address derived from
EVM transaction intent. Landing the mandatory field is one deliberate breaking spec bump.

### Mechanics

- The adapter derives the `Exclusive` key from typed intent during invocation preparation. The key
  is recorded on the existing invocation-prepared event payload; no new event family exists.
- Lane acquisition is the admission precondition of that same atomic commit: the store rejects an
  invocation-prepared carrying `(namespace, key)` while any other ledger holds that lane — another
  run's or the same run's. Parallel forward branches in one run race a shared key exactly like two
  runs do. The same ledger may re-prepare under a new invocation epoch with the same key; replay
  verifies key stability across epochs.
- A lane is held from invocation-prepared until the holding ledger records terminal evidence —
  confirmation observed, not-submitted proven, failure, or manual resolution covering that ledger —
  or until the holding run records its sealed terminal resolution. Run-terminal release is
  deliberate: a ledger that never crossed the boundary (frozen at prepared by the fence) records
  no ledger-terminal evidence at all, and an unresolvable ledger in a run that degraded through
  policy `FailWithoutAcdcClaim` has its operator path closed once the run is terminal; without
  run-terminal release both hold their lanes forever with no recourse. `FailedWithoutAcdcClaim`
  already publicly disclaims the resource state, and a peer acquiring a lane released this way can
  read the releasing run's terminal outcome and unresolved obligations in public status. A crashed
  or ambiguous holder in a live run keeps the lane — the external resource genuinely is in an
  unknown state — until evidence, operator authority, or the holder's sealed terminal resolution.
- Lane state is a derived cross-run projection, rebuildable like every other projection and
  bounded to non-terminal runs: a run's sealed terminal resolution releases all its lanes, so
  rebuild scans live runs only. Acquisition order is recorded commit order; there are no
  lane-control events and nothing to forge.
- The scheduler treats a held lane as a per-node blocked condition: that node waits, the rest of
  the frontier and all other runs proceed. No new `RunMode`; blocked-on-lane is projected detail
  inside `Forward`/`Remediating`. No fairness guarantee in this slice.
- `ExactTouchedSet` sequences nothing. The attempt emits the actual touched set as typed evidence
  on the existing receipt/confirmation evidence, schema-checked at admission. In this slice it is
  captured and replay-checked for presence and schema; the verifier that bounds compensation scope
  to the recorded set is deferred. Its value in this slice is that the evidence schema lands now
  and evidence accumulates for the deferred verifier to run against; it enforces nothing yet, and
  public status must not imply it does.
- Remediation ledgers acquire lanes under exactly the same rules: a compensation transaction races
  concurrent runs the same way a forward one does.

### Claim discipline

With `Exclusive` lanes MFM may claim: mutations declaring the same `(namespace, key)` are
serialized at the uncertainty boundary, within a run and across runs. MFM still may not claim AC/DC equivalence,
phantom freedom, or anything about keys that were never declared. Keys are adapter-derived
evidence: schema-checked and stability-checked, not framework-re-derived. Per-run replay verifies
key evidence; cross-run serialization itself is enforced at store admission and auditable from
recorded commit order, and a multi-run audit verifier is deferred.

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

The post-core resource milestone adds only: the `ResourceClaimSpec` and `ResourceNamespace` types,
the mandatory node-spec field and builder parameter, resource evidence on existing event payloads,
the derived lane projection and its admission precondition, and the public lane/claim status
fields.

Avoid in v1:

- new effect class hierarchy;
- per-node directive maps and obligation/manual ID namespaces;
- control events for derivable decisions;
- generic remediation handler traits, per-state compensation methods, dynamic policy callbacks;
- exposing store projection internals as authoring API;
- compatibility adapters for old run-status semantics;
- framework implementations of commutativity, escrow, predicate isolation, or reorg semantics.

## Implementation Milestones

### Milestone 0: contract PR

Purpose: land type skeletons and compile-time contracts without runtime behavior.

Scope:

- spec types: `SagaPolicySpec`, `remediations` collection, manual evidence spec;
- `SideEffectLedgerPurpose` and `ManualResolutionOutcome` types;
- extended `RunCompletionOutcome` and `RunMode` types;
- decode-posture audit: persisted spec and event payload deserialization must deny unknown
  fields, or the closed-types claim does not hold;
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
- the forward-progress fence and quiescence admission checks;
- `ManualResolutionRecorded` payload and the split admission preconditions (store-derivable plus
  runtime-constructed, per Store Semantics);
- extended `RunCompleted` admission preconditions;
- derived obligation and run-mode projections as pure functions of policy plus stream (in-memory
  and Postgres);
- append/rebuild projection parity tests.

Acceptance:

- `cargo test -p mfm-events`;
- `cargo test -p mfm-store`;
- the focused Postgres storage package tests.

### Milestone 3: runtime remediation

Purpose: prove runtime-owned saga transition.

Scope:

- derive `Remediating` from engaging events (non-retryable failure, forward ambiguity) after
  confirmed side effects;
- quiescence driving of past-boundary forward ledgers and forward output-cell materialization;
- eligibility classification at quiescence;
- sequential reverse-confirmation-order remediation scheduling;
- uniform unresolved handling into `ManualBlocked` or terminal;
- sealed `ResolveSagaTerminal` lifecycle node (lowered into every spec, exactly one).

Acceptance:

- two confirmed side effects, later failure, both compensated in reverse confirmation order;
- crash/resume at every boundary: after failure, between forward confirmation and forward output
  cell, after first remedial submission, after first remedial confirmation, before terminal
  resolution — no duplicated mutation;
- forward scheduler provably cannot select remediation nodes; remediation scheduler provably
  cannot select forward nodes (quiescence driving and output-cell materialization excepted as
  completion work);
- forward ambiguity with no other failure engages the saga and reaches `ManualBlocked` or a
  terminal;
- confirmation committed after the engaging event classifies owed and joins reverse confirmation
  order;
- ambiguous forward ledger at quiescence degrades per policy, never `Compensated`;
- boundary crossed but empty quiescent sets resolves `FailedWithoutAcdcClaim`, never vacuous
  `Compensated`.

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

### Milestone 6: cross-run resource claims

Purpose: prove that concurrency correctness across isolated runs is platform authority, not domain
convention. Starts only after Milestone 5 is done.

Scope:

- mandatory `ResourceClaimSpec` field on `ApplySideEffect` node specs (one breaking spec bump);
- resource key on invocation-prepared payloads; touched-set evidence on receipt/confirmation;
- derived cross-run lane projection and admission precondition (in-memory and Postgres);
- scheduler blocked-on-lane handling for forward and remediation frontiers;
- replay checks for key presence, schema, and per-ledger stability;
- public status for claims, keys, and lane blockage.

Acceptance:

- two runs declaring the same exclusive key sequence at the uncertainty boundary; unrelated keys
  proceed in parallel;
- parallel branches of one run declaring the same exclusive key sequence like two runs do;
- a crashed or ambiguous holder in a live run keeps the lane until resolved; manual resolution or
  the holder's sealed terminal resolution releases it;
- a lane held by a prepared-but-never-started ledger releases at the holding run's sealed
  terminal resolution;
- remediation ledgers acquire lanes identically;
- missing or schema-invalid key/touched-set evidence is rejected at admission and replay;
- `ManualOnly` nodes run unsequenced and public status carries no concurrency claim for them.

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
   `RunCompletionOutcome` rewrite. Removing `Failed` and `Cancelled` necessarily ripples through
   runtime history validation, store outcome handling, replay, and app in this commit so the
   workspace compiles; keep the semantic surface narrow even though the diff is wide.

5. `store: derive saga projections and preconditions`

   Derived obligation/run-mode projections as pure functions of policy plus stream; the
   forward-progress fence and quiescence checks; the store-derivable admission preconditions for
   remedial ledgers, manual evidence, and terminal outcomes (spec-aware preconditions arrive with
   the runtime commits); rebuild parity tests.

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

11. `spec: require resource claims on side-effect nodes`

    `ResourceClaimSpec`, `ResourceNamespace`, the mandatory node-spec field and builder parameter,
    hash-defining, with certification of schema presence. One deliberate breaking spec bump.
    Milestone 6 starts here.

12. `store: derive cross-run resource lanes`

    Resource key on invocation-prepared, touched-set evidence on receipt/confirmation, the derived
    lane projection, the cross-run admission precondition, and rebuild parity tests (in-memory and
    Postgres). Postgres lane admission must be genuinely serialized across concurrent committers
    (unique index or equivalent), not check-then-append.

13. `runtime: sequence exclusive lanes`

    Blocked-on-lane scheduling for forward and remediation frontiers, with the two-run sequencing
    and parallel-unrelated-keys tests.

14. `replay: verify resource evidence`

    Key presence/schema/stability and touched-set checks, with rejection tests. Public status for
    claims and lane blockage rides this or a final small app commit.

Do not merge commits that add a semantic shape without its corresponding rejection tests unless the
commit is a pure type skeleton explicitly marked as such. Do not combine runtime remediation,
replay verification, and public status in one commit.

## Test Matrix

The implementation is not credible until these tests exist.

| Area | Required tests |
| --- | --- |
| Builders | Unlinked forward side-effect node under compensating policy fails finalize; remediation handle unusable as forward node (compile-fail); out-of-scope remediation binding fails finalize. |
| Certification | Policy/graph variant disagreement rejected; remediation key to missing or non-side-effect node rejected; coverage gap rejected; non-side-effect-grade remediation node rejected; manual spec without schemas rejected; node-id overlap between `nodes` and `remediations` rejected; non-empty `remediations` under a non-compensating policy rejected; missing or duplicate `ResolveSagaTerminal` rejected. |
| Store | Remediation ledger requires confirmed forward ledger plus an engaging event; second remediation ledger for same forward key rejected; forward boundary-crossing events after an engaging event rejected (fence); manual evidence admitted only while derivably `ManualBlocked`, once, at quiescence; terminal outcome disagreeing with derived projection rejected; terminal or manual admission without quiescence rejected; projections rebuild from stream. |
| Runtime | Two confirmed effects then later failure remediate in reverse confirmation order; saga engages on forward ambiguity with no other failure; two simultaneous engaging failures anchor at the first in stream order; confirmation after the engaging event classifies owed; clean failure resolves `FailedWithoutAcdcClaim` under every policy variant; boundary crossed with empty quiescent sets resolves `FailedWithoutAcdcClaim`, never vacuous `Compensated`; ambiguous forward ledger at quiescence degrades per policy; remediation failure/ambiguity follows `on_remediation_unresolved` and stops remaining obligations; crash/resume at every remediation boundary including between forward confirmation and output cell; frontier separation proven both directions. |
| Replay | Compensated run verifies; `Compensated` with a missing remedial confirmation rejects; vacuous `Compensated` (empty owed set) rejects; remediation ledger with wrong forward linkage rejects; manual evidence schema mismatch rejects; terminal without quiescence rejects; fence violation rejects; terminal claim unsupported by recomputation rejects. |
| Public status | Status distinguishes all seven run modes and shows per-ledger obligation classification, including unresolved obligations under `FailedWithoutAcdcClaim`. |

Milestone 6 resource-claim tests:

| Area | Required tests |
| --- | --- |
| Store | Same `(namespace, key)` lane rejects any other ledger's invocation-prepared while held, same-run or cross-run; lane releases on each ledger-terminal evidence kind and on the holding run's sealed terminal resolution; lane projection rebuilds from non-terminal run streams; touched-set evidence is schema-checked at admission. |
| Runtime | Two runs on one wallet-nonce lane sequence; parallel branches of one run on one lane sequence; unrelated keys run in parallel; remediation ledgers acquire lanes under forward rules; an ambiguous holder blocks peers until manual resolution or the holder's sealed terminal resolution releases the lane; a prepared-but-never-started holder's lane releases at run terminal. |
| Replay | `Exclusive` ledger without a recorded key rejects; key unstable across invocation epochs rejects; `ExactTouchedSet` ledger without touched-set evidence rejects. |

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
- If the engagement fence or classification quiescence is weakened, two readers of one stream can
  derive different obligation sets, and replay loses recompute-and-compare exactness. The fence
  and quiescence rules are load-bearing for every derived claim.
- If manual resolution grows spec-authored outcomes, it becomes a generic escape hatch again.
- If the eligibility table is weakened (e.g., compensating from receipt-only evidence),
  `Compensated` stops being a sound claim.
- If cancellation is reintroduced without remediation-aware design, it reopens the forged-terminal
  hole the current history validation closed.
- Run-level-only policy may prove too coarse for real workflows; per-node overrides are the known
  next step and must come back as certified spec data, not callbacks.
- A stuck or ambiguous ledger in a live run holds its exclusive lane until evidence, operator
  authority, or the holder's sealed terminal resolution. That is correct — the external resource
  genuinely is in question — but it turns one stuck run into blocked peers; operators need the
  lane-holder visibility public status provides. Run-terminal release trades protection for
  liveness: a peer acquiring a lane released by a `FailedWithoutAcdcClaim` terminal races a
  resource in unknown state and must read the releasing run's public status.
- `Exclusive` keys are adapter-derived evidence: schema- and stability-checked but not
  framework-re-derived in this slice. A wrong adapter key silently weakens sequencing until
  re-derivation verifiers exist.
- The scheduler's input grows beyond "certified graph plus own run history" to include the derived
  lane view. Cross-run acquisition order is recorded commit order, not a deterministic function of
  specs; resume and rebuild must treat the lane projection like any other derived state.

## Deferred Work

- Per-node failure directive overrides and retry policy.
- Per-obligation manual resolution targeting, per-ledger operator lane release, and manual
  outcome catalogs.
- Run cancellation semantics under remediation.
- Concurrent remediation scheduling.
- Lane fairness/queueing, cross-run deadlock detection, and multi-run serialization audit
  verifiers.
- Deterministic re-derivation verifiers for `Exclusive` keys and touched-set bounding verifiers
  for compensation scope.
- `ApplyCompensation` effect class.
- Generic commutativity, escrow, predicate isolation, and phantom-proof framework semantics.
- Chain-specific reorg/finality verifiers and triggers.
- Continuation child-run execution beyond typed parent/child linkage.
- AC/DC-equivalence claims and domain verifier APIs.

## Definition Of Done For V1

V1 is done when MFM can run, persist, resume, replay, and inspect the two-mutation end-to-end slice
above, and the test matrix proves that compensation is certified runtime/store/replay behavior —
derived from certified policy plus append-only facts — rather than ordinary user-state bookkeeping.
