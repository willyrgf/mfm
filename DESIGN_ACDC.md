# DESIGN: certified saga remediation policy for MFM

Status: design-exploration record. The v1 scope and decisions formerly resolved here are
superseded by `RFC_ACDC_SAGA.md`; where the two disagree, the RFC wins. Sketches in this document
are design pressure for later slices, not current implementation contract.

Note: the RFC's second review pass departs from this document's sketches in two further ways. The
remediation control events sketched below were cut entirely — directive selection, obligation
open/close, and terminal state are derived projections whose timing is pinned by the RFC's
engagement fence and classification-quiescence rules — and saga admission authority is split
between spec-blind stream-derivable store checks and runtime-constructed spec-aware
preconditions, with replay as the policy backstop.

Companion problem statement: `PROBLEM_ACDC.md`.

## Purpose

`PROBLEM_ACDC.md` argues that MFM already has durable forward execution foundations, but cannot
claim AC/DC workflow semantics for arbitrary external systems. The realistic target is certified
saga semantics for external effects: failure directives, compensation, manual resolution, resource
claims, and concurrency evidence become certified typed execution semantics.

MFM can make stronger AC/DC-style claims only where it owns the affected transactional resource or
where the certified run records enough replay-verifiable evidence to prove the required
correctness properties. This design keeps the AC/DC pressure, but it does not pretend that durable
bookkeeping makes external systems atomic.

This document proposes the smallest architecture shape that moves MFM in that direction without
blowing up public API or LOC:

- keep `EffectClass` as the coarse authority boundary;
- add a certified run-level `RemediationPolicy`;
- add certified resource-claim derivation for mutating work;
- reuse the existing side-effect ledger protocol for remedial mutations where possible;
- make runtime/store own all durable failure, remediation, and terminal mode transitions.

## Core Position

Certified saga correctness is a property of a certified run, not a property of an individual state
type. Stronger AC/DC-style correctness is a scoped property of a certified run plus the resources
and proof contracts it controls or verifies.

State types should declare what they can do and what evidence shapes they produce. A certified run
should declare what failure policy applies to a specific graph, node, side-effect obligation, or
business boundary. Runtime and store should enforce that policy from append-only event evidence.

The important split is:

| Layer | Owns |
| --- | --- |
| `EffectClass` | What kind of authority a state may exercise. |
| State contracts | Typed intent, input, output, resource-claim derivation, and evidence shapes. |
| Certified run policy | What should happen when failure, conflict, finality, or rollback evidence appears. |
| Runtime | Deterministic mode transitions, frontier selection, and guarded commits. |
| Store | Append-only event admission, logical keys, resource indexes, projections, and preconditions. |
| Replay | Evidence-only verification of forward, remedial, and resource-claim history. |

Runtime/store already need to persist all semantic transitions through append-only typed events.
That should remain the durability foundation for certified saga remediation and any scoped
AC/DC-style claim. `ManagedPlatformWrite` should not become the mechanism that makes every state
durable. It should remain the effect class for explicitly managed platform writes, such as public
output, retention, artifacts, or redacted diagnostics.

## Design Principles

1. Policy is certified data, not app glue.

   Remediation policy and resource-claim derivation must be included in the certified execution
   spec or in certified spec-bound companion data. A dynamic callback, closure, CLI flag, or
   app-level handler cannot be semantic authority for resume or replay.

2. Invalid semantic combinations should be unrepresentable where practical.

   The public authoring API should use typed builders, branded handles, sealed constructors,
   effect-specific state traits, and closed enums so most missing-policy cases cannot be built.
   Certification remains necessary because persisted specs, registry inputs, and lowered data are
   hostile until verified, but certification should be a final authority check rather than the main
   place where ordinary authors discover invalid saga structure.

3. Policies are run-level, not state-local.

   A state may be reused in many workflows with different failure behavior. For example, one
   workflow may compensate a trade, another may replan the whole portfolio, and another may block for
   manual resolution. The state type should not hard-code those workflow choices.

4. Directives are closed and framework-understood.

   MFM should support a small enum of failure directives that runtime/store/replay understand. Domain
   specifics live in typed state configs, typed evidence, and verifier-backed correctness claims, not
   arbitrary control flow.

5. Remediation is forward-first.

   AC/DC cannot mean erasing or mutating prior events. It means appending certified remediation
   evidence and terminal resolution evidence.

6. Compensation is side-effect-grade.

   A remedial mutation needs the same core properties as a forward side effect: typed intent, typed
   idempotency input, durable claim/fencing, invocation boundary, receipt or confirmation evidence,
   ambiguity handling, and replay verification.

7. Resource conflicts are certified evidence, not hidden state behavior.

   State and adapter contracts may derive domain resource keys from typed intent, effect/capability
   authority, adapter identity, and recorded evidence, but any correctness-relevant resource key,
   operation id, touched set, predicate snapshot, finality proof, or commutativity claim must be
   certified and emitted as typed evidence.

8. Replay remains evidence-only.

   Replay must not call live transports to decide whether a workflow was compensated, manually
   resolved, or safe to retry.

## Proposed Model

### Effect classes

Keep the existing effect classes.

| Effect class | AC/DC interpretation |
| --- | --- |
| `Pure` | Can be recomputed and replayed freely if deterministic and no ambient IO leaks in. |
| `ReadExternal` | Can retry live read failures before fact recording. Once a fact is recorded, replay reads from evidence only. Fresh reads require a new attempt, replan, or continuation boundary. |
| `ManagedPlatformWrite` | Represents platform-managed writes only. It is not the durability mechanism for the workflow as a whole. |
| `ApplySideEffect` | External mutation governed by a side-effect ledger. Retry is safe only when evidence proves the mutation did not happen or when the certified directive permits a specific recovery path. |

### RemediationPolicy

Add a run-level remediation policy to certified runtime authority.

The policy should be generated by operations or builders during planning, lowered into the typed
execution spec, and verified by certification. For v1 it should live directly in
`TypedExecutionSpec` as hash-defining certified data, not in a separate companion digest and not in
node-local framework lifecycle metadata. This keeps runtime authority aligned with the existing
certified-spec boundary.

The policy should not be interpreted from raw user input at runtime. The builder API should make the
common invalid combinations unrepresentable: a side-effecting forward node should not be able to
reach a terminal public output without an attached directive, and a compensation directive should
require a typed obligation target before the program draft can be finalized.

Sketch of the spec placement:

```rust
pub struct TypedExecutionSpec {
    // existing hash-defining fields omitted
    pub remediation: RemediationPolicySpec,
}
```

Sketch:

```rust
pub struct RemediationPolicySpec {
    pub default_failure: FailureDirectiveSpec,
    pub node_overrides: BTreeMap<NodeId, FailureDirectiveSpec>,
    pub obligations: BTreeMap<RemediationObligationId, RemediationObligationSpec>,
    pub resources: BTreeMap<NodeId, ResourceClaimSpec>,
    pub triggers: Vec<RemediationTriggerSpec>,
    pub correctness: CorrectnessPolicySpec,
}
```

The public authoring API should be smaller than this internal shape. A builder can expose a narrow
surface such as:

```rust
program.remediation()
    .on_failure(node, FailureDirective::CompensateCompleted)
    .compensate(forward_node, compensation_node)
    .manual_on_ambiguity("operator_review");
```

The internal `RemediationPolicySpec` is a lowered representation, not the preferred authoring
surface. Public APIs should prefer typestate-style builders or branded handles such as
`SideEffectNode<NeedsDirective>`, `SideEffectNode<PolicyCovered>`, and
`RemediationObligation<HasVerifierOrManual>` where that keeps invalid states out of user code
without making ordinary workflows verbose.

### Failure directives

Use a closed directive enum. Initial variants should cover the situations discussed so far without
trying to encode every possible workflow paper concept.

```rust
pub enum FailureDirectiveSpec {
    RetrySameNode {
        max_attempts: u32,
    },
    CompensateCompleted {
        scope: CompensationScopeSpec,
        ordering: CompensationOrderingSpec,
    },
    ReplanFromStart {
        fresh_reads: FreshReadPolicySpec,
    },
    StartCertifiedContinuation {
        continuation: ContinuationSpec,
    },
    ManualResolution {
        reason_code: ManualResolutionReason,
    },
    FailWithoutAcdcClaim,
}
```

Notes:

- `RetrySameNode` is valid for pure/read states and for side effects only before the durable
  uncertainty boundary or after not-submitted evidence.
- `CompensateCompleted` opens obligations for completed side effects selected by certified graph and
  ledger evidence.
- `ReplanFromStart` must not silently mutate the old run into a new topology. It should create a new
  certified planning boundary, continuation run, or run epoch with explicit fresh-read policy.
- `StartCertifiedContinuation` is the clean path when runtime data must select new topology.
- `ManualResolution` is a durable run mode with typed operator evidence, not an out-of-band note.
- `FailWithoutAcdcClaim` is allowed only when the certified policy says the workflow may fail
  without a compensation or AC/DC-equivalence claim.

### Remediation obligations

A remediation obligation links a completed forward side effect to a certified remedial path.

To minimize new public API, the first design should model a remedial action as another certified
`ApplySideEffect` node that is dormant during forward execution and runnable only in remediation
mode. Runtime knows it is not an ordinary user state because the certified policy links it to a
forward side-effect ledger obligation.

The graph model should be one certified spec with two runtime frontiers: a forward frontier and a
remediation frontier. Remediation nodes can use normal `NodeSpec` records, but certification must
mark them remediation-only so the forward scheduler cannot run them. Framework lifecycle nodes may
open obligations, record terminal resolution, and validate sealed transitions; they must not invoke
domain side-effect runners.

Sketch:

```rust
pub struct RemediationObligationSpec {
    pub forward_node: NodeId,
    pub remediation_node: NodeId,
    pub strategy: RemediationStrategySpec,
    pub correctness: CorrectnessClaimSpec,
    pub required_forward_resource_evidence: ResourceEvidenceRequirementSpec,
    pub ambiguity_directive: FailureDirectiveSpec,
}
```

This avoids adding a new `ApplyCompensation` effect class initially. If the existing ledger is
reused, the ledger must carry a first-class persisted purpose such as `Forward` or
`Remediation { obligation_id, forward_ledger_key }`. That purpose must participate in ledger
identity, projection, preconditions, and replay verification. Without those fields, a dormant
remediation node is only an ordinary side-effect state with special scheduling, not certified saga
semantics.

If later experience shows that separating forward mutation from remedial mutation buys real clarity,
`ApplyCompensation` can be added as a specialization. It should not be the starting point.

### Triggers

Some remediation decisions are caused by later evidence, not the immediate failing state. Examples:

- an on-chain receipt later becomes invalid under a reorg/finality policy;
- a future confirmation state proves rollback;
- a downstream state discovers the workflow must replan from fresh reads;
- a side-effect submission becomes ambiguous and requires manual authority.

This should be modeled as certified trigger data, not as a future state directly mutating older
state history.

Sketch:

```rust
pub struct RemediationTriggerSpec {
    pub source_node: NodeId,
    pub evidence_class: TriggerEvidenceClass,
    pub target: RemediationTargetSpec,
    pub directive: FailureDirectiveSpec,
}
```

Runtime may act on a trigger only when the recorded evidence matches the certified trigger. The
older node or ledger is not retried because a later state asked for it. It is retried, compensated,
or escalated because certified policy plus append-only evidence require that transition.

### Resource claims and parallel runs

Runs have independent append-only histories, but the resources they mutate may overlap. MFM should
make that overlap explicit instead of relying on private state implementation behavior.

Sketch:

```rust
pub struct ResourceClaimSpec {
    pub namespace: ResourceNamespace,
    pub derivation: ResourceClaimDerivationSpec,
    pub operation_kind: ResourceOperationKind,
    pub concurrency: ResourceConcurrencySpec,
    pub actual_evidence_schema: SchemaId,
}

pub enum ResourceClaimDerivationSpec {
    FromTypedIntent,
    FromAdapterIdentity,
    FromIntentAndAdapter,
    FromRuntimeEvidence,
    Conservative(ResourceScopeSpec),
}

pub enum ResourceConcurrencySpec {
    Exclusive,
    Commutative,
    EscrowBounded,
    PredicateSnapshotRequired,
    ManualOnly,
    UnsequencedExternal,
}
```

Runtime/store can conservatively sequence `Exclusive` claims when a concrete key can be derived
before mutation. For example, an EVM transaction state and adapter can derive an account nonce lane
from chain id and sender; workflows using the same lane wait for nonce advancement while workflows
using unrelated senders continue in parallel. When the exact touched set is only known after
execution, the attempt must emit typed resource evidence. Replay verifies that evidence against the
certified resource-claim contract.

If a state cannot derive or prove enough resource information, the workflow can still run durably,
but MFM should not mark the terminal outcome as platform-certified concurrent correctness.

The useful lanes are:

- MFM-owned resources: runtime/store can enforce stronger serialization and atomicity.
- typed external resources with evidence: MFM can provide certified saga correctness and scoped
  AC/DC-style claims when replay-verifiable proof is sufficient.
- opaque external resources: MFM can provide durable execution, compensation attempts, and manual
  resolution, but not platform-proven concurrency correctness.

## Runtime Modes

The current "blocked" scheduler result is not enough for certified saga remediation or scoped
AC/DC-style claims. Run status needs durable semantic phases derived from the stream.

Proposed mode vocabulary:

| Mode | Meaning |
| --- | --- |
| `Forward` | Normal certified graph execution. |
| `RecoveringSideEffect` | Runtime is resolving an uncertain forward side-effect phase without duplicating mutation. |
| `Remediating` | Forward execution has stopped and remediation obligations are being scheduled. |
| `Replanning` | The run has reached a certified planning boundary for fresh reads or continuation. |
| `ManualBlocked` | Runtime requires typed operator evidence before it can continue or terminate. |
| `Completed` | Forward workflow reached successful certified output. |
| `Compensated` | Required saga remediation obligations completed with certified evidence. This is not automatically an AC/DC-equivalent outcome unless the correctness proof contract says so. |
| `ManuallyResolved` | Operator evidence completed the certified manual path. |
| `IrreversibleBlocked` | Failure crossed a certified irreversible boundary and cannot claim compensation. |
| `FailedWithoutAcdcClaim` | Terminal failure allowed by certified policy without compensation or AC/DC-equivalence claim. |

These should be stream-derived run projections, not app-layer status strings.

### Manual resolution evidence

Manual resolution should be a narrow typed protocol, not a generic escape hatch. The minimal useful
shape is:

```rust
pub struct ManualResolutionEvidenceSpec {
    pub target: ManualResolutionTargetSpec,
    pub reason_code: ManualResolutionReason,
    pub allowed_outcomes: Vec<ManualResolutionOutcomeSpec>,
    pub evidence_schema: SchemaId,
    pub authorization: ManualResolutionAuthorizationSpec,
    pub redaction_policy: RedactionPolicySpec,
}
```

The corresponding event should record the selected outcome, evidence artifact reference, signed
authorization proof artifact reference, and optional redaction-safe note. It should not allow an
operator to invent an uncertified terminal meaning. Certification decides which manual outcomes are
legal for the run or obligation, which evidence schema is accepted, which verifier is trusted, and
which operator authority snapshot is bound. The operator supplies typed evidence and a signed
authorization proof for one of those outcomes.

Registry-backed authority is used only at certification time. The certified spec and certificate
carry the replay authority: schema role grants, verifier identity, authority snapshot evidence, and
registry digest. Replay verifies manual resolution from the certified spec/certificate, stream, and
retained artifacts only; it must not call live signer or registry sources. `ManuallyResolved` means
an authorized manual decision was recorded, not that MFM independently proved external domain truth.

## Store And Event Shape

The store should continue to own event envelopes, sequence numbers, logical keys, preconditions, and
projections. Saga remediation adds new event families, but should avoid duplicating the entire
side-effect protocol if possible.

Preferred direction:

1. Generalize side-effect ledger purpose.

   Existing side-effect ledger phases can apply to both forward and remedial mutations if the ledger
   carries a persisted purpose such as `Forward` or
   `Remediation { obligation_id, forward_ledger_key }`, plus obligation linkage for remedial
   mutations. Store validation should reject a remediation ledger unless the linked obligation has
   been opened and the linked forward ledger is confirmed or otherwise policy-eligible.

2. Add remediation-control events.

   These describe mode transitions and obligations, not protocol-level submission/receipt phases.

3. Add resource evidence events or fields.

   These record declared and actual resource keys, operation ids, touched sets, predicate snapshots,
   finality evidence references, and verifier ids when those are part of the certified correctness
   claim.

Minimal event families:

```rust
FailureDirectiveSelected
RemediationObligationOpened
RemediationObligationClosed { outcome }
ManualResolutionRequested
ManualResolutionRecorded
ResourceEvidenceRecorded
RunTerminalResolved
```

Superseded: `RFC_ACDC_SAGA.md` keeps only `ManualResolutionRecorded` and the extended
`RunCompleted` from this list. `FailureDirectiveSelected`, `RemediationObligationOpened`,
`RemediationObligationClosed`, `ManualResolutionRequested`, `ResourceEvidenceRecorded`, and
`RunTerminalResolved` are not events; they are derived projections (resource evidence rides
existing side-effect payloads), with derivation timing pinned by the RFC's engagement fence and
classification-quiescence rules.

The side-effect submission, receipt, confirmation, ambiguity, and failure phases should remain the
normal side-effect event protocol, distinguished by ledger purpose. The exact names can change. The
contract should not: every semantic transition is append-only, store-validated, projected, and
replay-verifiable.

## Authoring And Certification Rules

The preferred design is to make side-effecting workflows that leave failure, remediation, or
claimed AC/DC-equivalence behavior ambiguous unrepresentable through the normal operation/program
builders. Certification still must reject ambiguous lowered specs because certification is the
authority boundary for persisted bytes, registry-derived descriptors, cross-crate builders, and any
future migration or repair path.

Initial rules to encode in builders first and verify in certification:

- a reachable `ApplySideEffect` before successful terminal output carries a policy-covered failure
  path before the program draft can be finalized;
- a mutating node exposes a certified resource-claim derivation class, even if the class is
  `UnsequencedExternal` or `ManualOnly`;
- `CompensateCompleted` construction requires at least one certified remediation obligation;
- a remediation obligation can target only a side-effect-grade or manual-only remediation node;
- a remediation obligation carries either a correctness proof contract or an explicit
  manual/fail-without-claim degradation;
- an irreversible node is modeled by topology or policy so it appears after fallible compensatable
  work, splits the workflow at a continuation boundary, or declares a manual/irreversible terminal
  path;
- continuation/replan construction identifies where fresh facts are allowed and which old facts are
  no longer semantic authority for the new plan;
- manual path construction requires typed evidence shape and terminal outcome.

Certification should verify the same invariants from the lowered spec and reject any manually
constructed or stale persisted shape that bypassed the builder API.

## Correctness Claims

The kernel should name generic correctness classes, but not pretend to prove domain semantics it
cannot prove.

Potential classes:

| Class | Meaning |
| --- | --- |
| `ExactTouchedSet` | Compensation is limited to objects recorded in forward evidence. |
| `Commutative` | Forward and remedial operations commute with allowed concurrent work. |
| `EscrowBounded` | Domain proves a bounded resource invariant rather than restoring old values. |
| `PredicateSnapshot` | Forward evidence recorded the predicate/object set needed to avoid phantoms. |
| `IsolationProof` | Domain evidence proves no conflicting concurrent workflow affects the obligation. |
| `DomainVerifier` | A certified replay verifier checks domain-specific correctness from typed evidence. |
| `ManualOnly` | The platform cannot prove correctness and requires operator authority. |
| `Irreversible` | No compensation claim is allowed after this boundary. |

If runtime/replay cannot verify a declared correctness claim, the run must not be marked
`Compensated` with an AC/DC-equivalence claim. It should block for manual resolution or fail without
an AC/DC claim according to certified policy.

Opaque assertions are not enough for platform-certified correctness. A domain-specific proof is
acceptable only when certification names the verifier, the verifier inputs are typed evidence, and
replay can run the verifier without live IO. Otherwise the policy should use `ManualOnly` or
`FailWithoutAcdcClaim`.

Sketch:

```rust
pub struct CorrectnessClaimSpec {
    pub class: CorrectnessClassSpec,
    pub verifier: CorrectnessVerifierSpec,
    pub evidence_inputs: Vec<EvidenceInputSpec>,
    pub output_schema: SchemaId,
}
```

V1 should split classes into framework-verifiable and domain-verifiable groups:

| Group | V1 classes |
| --- | --- |
| Framework-verifiable | MFM-owned resource serialization, concrete-key `Exclusive`, obligation linkage, once-only ledger progression, reverse-order remediation scheduling, and typed evidence presence/hash/schema checks. |
| Domain-verifier or manual | `Commutative`, `EscrowBounded`, `PredicateSnapshot`, `IsolationProof`, chain finality/reorg correctness, and any claim involving external-world truth. |

## Reads And Replanning

Read facts are run evidence. Replay of the same run should use the recorded fact.

If a workflow policy says "start over from the beginning" after a partial portfolio rebalance, that
should not mean rerunning the same certified graph while pretending old facts never existed. It
should mean one of:

- open a certified continuation with fresh read authority;
- create a new run linked to the failed/remediated parent;
- enter a certified replan mode that records which prior facts are superseded for the new epoch.

The old run history remains append-only authority. Fresh reads are new evidence, not mutation of old
facts.

Certified continuations should be modeled as child runs. The parent run should append events such as
`ContinuationRunStarted` and `ContinuationRunResolved` that link a child run id, child spec hash,
target obligation or replan boundary, and terminal child outcome evidence. The parent remains in
`Replanning` or `ManualBlocked` until certified child evidence resolves the parent obligation. The
child run must never mutate parent history directly.

## Irreversible Boundaries

Irreversibility should be represented in both state/adapter metadata and run policy.

State and adapter contracts say whether an effect can become final or irreversible for a domain.
Run policy says what that means in this workflow: place the irreversible effect after fallible
compensatable work, split at a continuation boundary, require manual resolution, or allow
`FailWithoutAcdcClaim`. Certification should reject any policy that claims compensation or
AC/DC-equivalent backout across an irreversible boundary without a replay-verifiable proof contract.

## On-Chain Effects

On-chain confirmation evidence is not automatically an AC/DC finality claim.

The run policy and state evidence must distinguish:

- not submitted and safe to retry;
- submitted but not confirmed;
- included but not final under a declared finality policy;
- final and compensatable only by another transaction;
- final and irreversible for this workflow.

Reorg or rollback evidence should be modeled as a certified trigger that opens recovery,
compensation, replan, or manual resolution. It should not mutate historical confirmation events.
Adapters may collect the evidence, but the certified policy and replay-visible evidence determine
what terminal claim the run may make.

## Public API Minimization

The public API should grow in two places only:

1. Planning/certification builders get a run-level remediation surface and reuse state/adapter
   resource-claim contracts where possible.
2. Inspection/status surfaces expose durable remediation modes, obligations, and relevant resource
   conflicts.

Breaking public status changes are acceptable. The app/CLI/API surface should expose a semantic
`RunMode` vocabulary rather than preserving the old absent/started/completed-only model. Store-level
coarse `RunState` may remain as an internal precondition helper if useful, but it should not be the
user-facing status contract for certified saga runs.

Avoid initially:

- adding a new effect class;
- adding a broad custom error handler trait;
- making every state implement compensation methods;
- exposing store projection internals as authoring API;
- putting dynamic remediation closures into app or binary code.

If the first vertical slice can reuse existing `ApplySideEffect` nodes as remediation nodes, most
new work stays in spec, certification, runtime, store, replay, and docs.

## V1 Scope

V1 scope, frozen decisions, semantic rules, and the implementation slice are defined in
`RFC_ACDC_SAGA.md`.
