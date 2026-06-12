# RFC: certified saga vertical slice for MFM

Status: ready for engineering planning.

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

1. a certified run-level failure/remediation policy;
2. side-effect-grade remedial mutations linked to completed forward side effects;
3. append-only remediation, manual-resolution, resource-evidence, and terminal-resolution events;
4. runtime-owned transition from forward failure to remediation or manual resolution;
5. store-owned projections and preconditions for remediation authority;
6. replay verification without live IO;
7. public status that reports semantic run modes and unresolved obligations.

Stronger AC/DC-style claims are allowed only where MFM owns the affected transactional resource or
where replay can verify a certified domain proof from typed evidence.

## Non-Goals

- Do not claim full AC/DC for arbitrary external systems.
- Do not add a new `ApplyCompensation` effect class in v1.
- Do not add dynamic remediation callbacks, closures, CLI handlers, or app-level policy hooks.
- Do not make every state implement compensation methods.
- Do not implement generic commutativity, escrow, predicate isolation, or chain reorg semantics in
  the kernel.
- Do not preserve the old absent/started/completed public status model as the user-facing contract.
- Do not hide compensation as ordinary user states that runtime/store/replay cannot recognize.

## Architectural Decision

The v1 implementation should be a certified saga implementation, not a durable bookkeeping layer.

All semantic transitions must be persisted as append-only typed events. Runtime and store own durable
mode transitions. App, CLI, and REST surfaces may display or route these states, but they are not
semantic authority.

Layer ownership:

| Layer | V1 responsibility |
| --- | --- |
| `mfm-spec` | Hash-defining remediation policy, directives, obligations, resource claims, correctness claims, and remediation-only node metadata. |
| `mfm-certify` | Builder/lowered-spec validation. Reject ambiguous side-effecting workflows. |
| `mfm-events` | Typed remediation, manual-resolution, resource-evidence, continuation, and terminal-resolution event payloads. |
| `mfm-store` | Event admission, preconditions, remediation projections, run-mode projection, side-effect ledger purpose validation. |
| `mfm-runtime` | Failure-mode transition, obligation opening, remediation frontier scheduling, manual/fail terminal handling. |
| `mfm-replay` | Evidence-only validation of remediation and resource history. |
| `mfm-app` | Verified assembly and public status reporting. No remediation semantics. |
| storage crates | Durable persistence for new typed events/projections. No domain behavior. |

## Frozen V1 Decisions

These decisions are intentionally closed for the first implementation slice.

1. `RemediationPolicySpec` lives directly under `TypedExecutionSpec` as hash-defining certified data.
2. Failure directives and resource-claim decisions are run-level policy.
3. State and adapter contracts declare derivation and evidence capabilities.
4. V1 uses one certified spec with a forward frontier and a remediation frontier.
5. Remediation-only nodes are normal certified nodes that the forward scheduler cannot run.
6. V1 reuses `ApplySideEffect` for remediation.
7. Side-effect ledgers get persisted purpose/linkage, e.g. `Forward` or
   `Remediation { obligation_id, forward_ledger_key }`.
8. Remediation control events are minimal; submission/receipt/confirmation/ambiguity/failure stay in
   the side-effect protocol with ledger purpose.
9. Manual resolution is typed evidence with certified allowed outcomes.
10. Domain correctness that depends on external truth requires a certified replay verifier or
    degrades to `ManualOnly` or `FailWithoutAcdcClaim`.
11. Continuations are child runs linked by append-only parent events.
12. Irreversible boundaries are both state/adapter metadata and run policy.
13. Public status exposes semantic `RunMode`; coarse store `RunState` may remain internal.

## V1 Data Model

The exact Rust names can change during implementation, but the semantic shape should not.

### Spec

```rust
pub struct TypedExecutionSpec {
    // existing hash-defining fields omitted
    pub remediation: RemediationPolicySpec,
}

pub struct RemediationPolicySpec {
    pub default_failure: FailureDirectiveSpec,
    pub node_overrides: BTreeMap<NodeId, FailureDirectiveSpec>,
    pub obligations: BTreeMap<RemediationObligationId, RemediationObligationSpec>,
    pub resources: BTreeMap<NodeId, ResourceClaimSpec>,
    pub triggers: Vec<RemediationTriggerSpec>,
    pub correctness: CorrectnessPolicySpec,
}

pub enum FailureDirectiveSpec {
    RetrySameNode { max_attempts: u32 },
    CompensateCompleted {
        scope: CompensationScopeSpec,
        ordering: CompensationOrderingSpec,
    },
    ReplanFromStart { fresh_reads: FreshReadPolicySpec },
    StartCertifiedContinuation { continuation: ContinuationSpec },
    ManualResolution { reason_code: ManualResolutionReason },
    FailWithoutAcdcClaim,
}

pub struct RemediationObligationSpec {
    pub forward_node: NodeId,
    pub remediation_node: NodeId,
    pub strategy: RemediationStrategySpec,
    pub correctness: CorrectnessClaimSpec,
    pub required_forward_resource_evidence: ResourceEvidenceRequirementSpec,
    pub ambiguity_directive: FailureDirectiveSpec,
}
```

### Side-effect ledger purpose

The side-effect ledger must distinguish forward and remedial mutations.

```rust
pub enum SideEffectLedgerPurpose {
    Forward,
    Remediation {
        obligation_id: RemediationObligationId,
        forward_ledger_key: SideEffectLedgerKey,
    },
}
```

This purpose must participate in event validation, projections, preconditions, replay, and any
ledger identity logic needed to prevent mixing forward and remedial evidence.

### Run modes

Public status should expose semantic run modes:

```rust
pub enum RunMode {
    Forward,
    RecoveringSideEffect,
    Remediating,
    Replanning,
    ManualBlocked,
    Completed,
    Compensated,
    ManuallyResolved,
    IrreversibleBlocked,
    FailedWithoutAcdcClaim,
}
```

`Completed` means successful forward output. `Compensated` means certified saga remediation
obligations completed. It is not automatically an AC/DC-equivalent outcome unless the correctness
claim says replay can verify that stronger property.

### Minimal event families

V1 should add remediation-control events and reuse side-effect events for remedial IO:

```text
FailureDirectiveSelected
RemediationObligationOpened
RemediationObligationClosed { outcome }
ManualResolutionRequested
ManualResolutionRecorded
ResourceEvidenceRecorded
RunTerminalResolved
ContinuationRunStarted
ContinuationRunResolved
```

Implementation may rename these, but it must preserve the separation:

- control events describe policy decisions and obligations;
- side-effect events describe intent, invocation, submission, receipt, confirmation, ambiguity, and
  failure for both forward and remedial ledgers.

### Manual resolution evidence

Manual resolution must be bounded and typed.

```rust
pub struct ManualResolutionEvidenceSpec {
    pub target: ManualResolutionTargetSpec,
    pub reason_code: ManualResolutionReason,
    pub allowed_outcomes: Vec<ManualResolutionOutcomeSpec>,
    pub operator_identity_ref_schema: SchemaId,
    pub evidence_schema: SchemaId,
    pub redaction_policy: RedactionPolicySpec,
}
```

Manual events should record selected outcome, operator identity reference, evidence artifact, and
optional redaction-safe note. They must not allow uncatalogued terminal outcomes.

### Resource claims

V1 should implement only the small resource-claim subset needed for proof-of-reality:

```rust
pub enum ResourceConcurrencySpec {
    Exclusive,
    ExactTouchedSet,
    ManualOnly,
}
```

`Exclusive` is framework-verifiable when a concrete key is derived before mutation. `ExactTouchedSet`
is evidence-verifiable when the state/adapter emits the actual touched set. `ManualOnly` is the
degradation path when MFM cannot prove enough.

Do not implement generic `Commutative`, `EscrowBounded`, `PredicateSnapshotRequired`, or
`IsolationProof` as framework semantics in v1.

## Certification Rules

Certification should be the authority boundary for lowered specs, but normal authors should hit
typed builder constraints before certification where practical.

V1 certification must reject:

- reachable `ApplySideEffect` forward nodes that can reach successful terminal output without a
  policy-covered failure path;
- `CompensateCompleted` directives without at least one remediation obligation;
- remediation obligations targeting ordinary forward-only nodes;
- remediation-only nodes reachable from the forward frontier;
- remediation obligations without correctness claim or explicit manual/fail-without-claim
  degradation;
- side-effect nodes without a resource-claim class, even if the class is `ManualOnly`;
- compensation claims across irreversible boundaries without replay-verifiable proof;
- manual resolution paths without typed evidence shape and allowed outcomes;
- stale or manually constructed specs that bypass builder invariants.

## Runtime Semantics

V1 runtime behavior should prove one complete saga path.

Happy remediation path:

1. Run executes forward graph.
2. A forward `ApplySideEffect` reaches confirmation.
3. A later node fails non-retryably.
4. Runtime selects a certified failure directive and appends `FailureDirectiveSelected`.
5. Runtime enters `Remediating`.
6. Runtime opens obligations for completed eligible side effects.
7. Runtime schedules linked remediation-only `ApplySideEffect` nodes in reverse dependency order.
8. Remedial ledgers execute through the existing side-effect protocol with purpose `Remediation`.
9. Runtime closes obligations when remedial confirmation evidence is recorded.
10. Runtime appends terminal resolution and projects `Compensated`.

Manual path:

1. Runtime detects ambiguity or a policy-declared manual condition.
2. Runtime appends `ManualResolutionRequested`.
3. Run projects `ManualBlocked`.
4. Operator supplies certified typed evidence.
5. Store admits `ManualResolutionRecorded`.
6. Runtime/store project `ManuallyResolved`, `FailedWithoutAcdcClaim`, or another certified allowed
   terminal outcome.

Fail-without-claim path:

1. Failure occurs and certified policy allows no saga/ACDC claim.
2. Runtime appends terminal resolution with `FailedWithoutAcdcClaim`.
3. Public status must make clear that no compensation or AC/DC-equivalence claim is being made.

## Store Semantics

Store must remain append-only authority. Projections are rebuildable caches.

V1 store work:

- persist new event payloads;
- add run-mode projection;
- add remediation obligation projection;
- add manual-resolution projection;
- add resource-evidence projection;
- add side-effect ledger purpose to projections;
- enforce remediation ledger linkage preconditions;
- enforce once-only obligation open/close;
- reject closing an obligation before required remediation evidence exists;
- reject terminal resolution while obligations are unresolved unless policy allows manual or
  fail-without-claim terminal state.

The existing coarse `RunState` may remain for commit preconditions, but it must not be the public
semantic status model.

## Replay Semantics

Replay must not call live transports.

V1 replay work:

- index remediation-control events;
- index side-effect evidence by ledger purpose;
- verify remediation ledgers link to opened obligations and eligible forward ledgers;
- verify obligation close events match certified policy;
- verify manual resolution evidence shape and allowed outcome;
- verify resource evidence presence/hash/schema;
- reject `Compensated` terminal resolution when required obligations or evidence are missing.

Domain truth remains outside framework proof unless a certified replay verifier is named and all
inputs are recorded typed evidence.

## Public Surfaces

App/CLI/API status should expose:

- `RunMode`;
- selected failure directive, when any;
- opened obligations and their status;
- linked forward and remediation ledger keys;
- manual resolution reason and required evidence schema;
- terminal resolution and whether it carries only certified saga semantics or a scoped
  AC/DC-style claim.

This is a breaking public status change. It is allowed.

## Implementation Milestones

### Milestone 0: contract PR

Purpose: land type skeletons and compile-time contracts without runtime behavior.

Scope:

- add spec types for remediation policy, directives, obligations, resource claims, correctness
  claims, and manual evidence;
- add event type skeletons;
- add ledger purpose type;
- add run mode type;
- add rustdoc explaining saga-only external semantics.

Acceptance:

- `cargo check --workspace`;
- targeted unit tests for canonical JSON/spec hash shape if the new fields are hash-defining;
- no runtime behavior changes yet.

### Milestone 1: certification and builders

Purpose: make ambiguous side-effecting specs unrepresentable where practical and rejected when
lowered.

Scope:

- builder surface for attaching remediation policy;
- remediation-only node marking;
- certification rules from this RFC;
- negative tests for missing policy, missing obligation, remediation node reachable from forward
  frontier, and manual path without typed evidence.

Acceptance:

- `cargo test -p mfm-certify`;
- relevant program/spec UI or integration tests.

### Milestone 2: events and store projections

Purpose: persist remediation authority.

Scope:

- event payloads and schema descriptors;
- in-memory store projection updates;
- Postgres typed storage support;
- side-effect ledger purpose projection;
- obligation/run-mode/manual/resource projections;
- preconditions for obligation lifecycle and remediation ledger linkage.

Acceptance:

- `cargo test -p mfm-events`;
- `cargo test -p mfm-store`;
- `cargo test -p mfm-stream-store-postgres` or the focused Postgres storage package name used by
  the workspace;
- append/rebuild projection parity tests.

### Milestone 3: runtime remediation frontier

Purpose: prove runtime-owned saga transition.

Scope:

- detect non-retryable failure after confirmed side effect;
- select certified directive;
- open obligations;
- enter `Remediating`;
- schedule remediation-only nodes in reverse dependency order;
- close obligations and terminally resolve as `Compensated`;
- block as `ManualBlocked` when required.

Acceptance:

- runtime test for confirmed side effect followed by later failure and successful compensation;
- crash/resume test at each boundary: directive selected, obligation opened, remediation submitted,
  remediation confirmed, terminal resolution pending;
- test that forward scheduler never runs remediation-only nodes.

### Milestone 4: replay

Purpose: make the compensated run independently verifiable.

Scope:

- replay broker indexes remediation events and side-effect purpose;
- verifier checks obligation/ledger linkage;
- verifier rejects missing remediation evidence;
- verifier rejects live-IO-only correctness claims.

Acceptance:

- replay test for compensated run;
- replay rejection tests for forged obligation close, wrong remediation ledger, missing manual
  evidence, and compensated terminal state without required confirmation.

### Milestone 5: public status

Purpose: expose semantics to users and operators.

Scope:

- app status maps stream/projections to `RunMode`;
- CLI/API output reports obligations and manual requirements;
- docs update for public status contract.

Acceptance:

- app tests for `Forward`, `Remediating`, `ManualBlocked`, `Compensated`,
  `FailedWithoutAcdcClaim`;
- CLI JSON contract tests if CLI output changes.

### Milestone 6: minimal resource claims

Purpose: prove concurrency correctness is not only crash recovery.

Scope:

- implement `Exclusive`, `ExactTouchedSet`, and `ManualOnly`;
- add resource evidence event/projection;
- sequence conflicting `Exclusive` claims when concrete keys are known;
- require manual/fail degradation when resource evidence is insufficient.

Acceptance:

- two runs sharing one exclusive resource key sequence correctly;
- unrelated resource keys proceed independently;
- exact touched-set evidence is replay-verified;
- missing touched-set evidence prevents platform-certified concurrent correctness.

## Test Matrix

The implementation is not credible until these tests exist.

| Area | Required tests |
| --- | --- |
| Certification | Missing policy rejected; missing obligation rejected; remediation-only node cannot be forward-reachable; manual path without typed evidence rejected; irreversible compensation claim rejected. |
| Store | Obligation opens once; obligation cannot close before eligible evidence; remediation ledger requires opened obligation; terminal resolution blocked with unresolved obligations; projections rebuild from stream. |
| Runtime | Forward confirmation then later failure enters `Remediating`; remediation runs in reverse dependency order; crash/resume works at each remediation boundary; ambiguity enters `ManualBlocked`; fail-without-claim is explicit. |
| Replay | Compensated run verifies; missing remediation confirmation rejects; forged obligation close rejects; wrong forward ledger linkage rejects; manual evidence schema mismatch rejects. |
| Public status | Status distinguishes `Completed`, `Compensated`, `ManualBlocked`, `ManuallyResolved`, `IrreversibleBlocked`, and `FailedWithoutAcdcClaim`. |
| Resource claims | Same exclusive key sequences; different keys run independently; exact touched set is required for touched-set claim; missing resource evidence degrades to manual/fail. |

## First End-to-End Slice

Build one proof workflow:

```text
Pure setup
  -> ApplySideEffect forward mutation
  -> Pure or ReadExternal state that fails non-retryably
  -> Remediation-only ApplySideEffect compensation
  -> terminal Compensated run mode
```

Required proof:

- forward side effect reaches confirmation;
- later failure does not produce ordinary failed completion;
- runtime opens a certified remediation obligation;
- remediation side effect uses purpose `Remediation`;
- crash/resume does not duplicate forward or remediation mutation;
- replay verifies the run without live IO;
- public status reports `Compensated` with obligation evidence.

## Risks

- If remediation policy is not hash-defining certified spec data, resume/replay authority becomes
  ambiguous.
- If ledger purpose is not persisted and validated, remediation is only scheduling convention.
- If manual resolution accepts untyped outcomes, it becomes a generic escape hatch.
- If store projections become semantic authority, append-only correctness is weakened.
- If public status remains absent/started/completed, operators cannot distinguish failure,
  compensation, manual resolution, or fail-without-claim.
- If resource claims are skipped, the implementation proves crash recovery but not concurrency
  correctness.

## Deferred Work

- `ApplyCompensation` effect class.
- Generic commutativity and escrow semantics.
- Predicate isolation and phantom-proof framework semantics.
- Chain-specific reorg/finality verifiers.
- Continuation child-run execution beyond typed parent/child linkage.
- Broad public API polish after the vertical slice is proven.

## Definition Of Done For V1

V1 is done when MFM can run, persist, resume, replay, and inspect the first end-to-end slice above,
and the test matrix proves that compensation is certified runtime/store/replay behavior rather than
ordinary user-state bookkeeping.

