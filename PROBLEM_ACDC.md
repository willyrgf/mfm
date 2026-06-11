# PROBLEM: AC/DC workflow guarantees for MFM

Status: problem statement for the next MFM architecture attack.

Source context: Stonebraker, Zhou, Kraft, and Li, "Consistency and Correctness in
Data-Oriented Workflow Systems" (CIDR 2026 draft, local PDF
`/Users/willyrgf/Downloads/p9-stonebraker.pdf`).

## One Sentence

MFM has the foundations for durable typed workflow execution, but it cannot yet claim AC/DC
workflow semantics because failed multi-step runs with completed external side effects do not have a
first-class, certified, durable, correctness-aware compensation/backout model.

## Why This Matters

MFM workflows are not just local computations. They can observe external systems, submit
transactions, call services, write artifacts, and produce public outputs. Durable execution by
itself handles the forward path: if the process crashes, MFM can rebuild typed authority from the
append-only run stream and resume from committed evidence.

That is not enough for update-oriented workflows. If a run confirms one side effect and a later
state fails, MFM must not merely preserve a durable record of the partial result. The platform must
drive the run toward one of these outcomes:

- the workflow completes through the primary path;
- the workflow completes through a certified alternate path;
- the workflow is backed out or compensated so the visible state is equivalent to the workflow never
  starting, within the declared correctness model;
- the workflow is durably blocked for manual resolution because automatic correctness cannot be
  guaranteed.

Anything else leaves users with the same "oops logic" AC/DC is meant to remove.

## AC/DC In MFM Terms

AC/DC extends ACID-style expectations from one transaction to an entire workflow. For MFM, the terms
must mean the following.

Atomicity:
Every certified run must either reach a successful terminal output, reach a certified alternate
terminal output, or reach a terminal compensated/manual-resolution state. A failed run with
unresolved confirmed side effects is not atomically complete.

Consistency:
Each side-effecting workflow must declare enough domain semantics to restore required invariants
when the forward path cannot complete. A compensation that works only in the happy single-user case
is not enough.

Durability:
Forward steps, alternate steps, compensation steps, recovery probes, manual-resolution records, and
terminal backout decisions must all be append-only typed workflow events. Compensation must be
once-and-only-once in the same sense as forward side effects.

Correctness:
Backout or compensation must account for concurrent workflows. The target outcome is equivalent to
removing the failed workflow and then applying the other completed concurrent workflows that should
remain. This is the hard part: simple inverse operations can be wrong when predicates, reads,
intervening writes, or external systems create phantom and cascading-backout cases.

## What MFM Already Has

MFM already has important AC/DC prerequisites:

- certified typed execution specs are the only semantic runtime contract;
- run streams are append-only and authoritative;
- store commits are atomic at the event batch level;
- replay and resume rebuild authority from stored certified specs plus run streams;
- side effects have typed intent, idempotency input, durable ledger events, receipt or recovery
  evidence, claim fencing, invocation epochs, and ambiguity handling;
- live replay is forbidden, so replay authority is evidence-only;
- binaries remain transport-only and cannot smuggle workflow semantics around typed authority.

These are durable execution foundations. They are not yet workflow atomicity/correctness.

## The Gap

Current MFM side-effect semantics are forward-side-effect semantics. They model how to prepare,
submit, observe, recover, and confirm a mutation. They do not model what the platform must do when a
later workflow node fails after one or more prior side effects were already confirmed.

The missing pieces are:

- no first-class compensation contract linked to a forward side-effect contract;
- no certified reverse-order compensation schedule for completed forward steps;
- no typed compensation intent, idempotency input, receipt, confirmation, or failure phases;
- no store projection that can answer whether a run is fully compensated, partially compensated,
  blocked, or manually resolved;
- no terminal run phase that distinguishes "failed with unresolved effects" from "compensated" or
  "manual resolution recorded";
- no certification rule that rejects side-effecting workflows without an explicit backout strategy;
- no correctness model for concurrent compensation, predicate phantoms, cascading effects,
  commutative updates, escrow-like updates, or domain assertions;
- no policy for irreversible side effects beyond relying on workflow authors to be careful;
- no platform-level alternate-step directive semantics for failure handling as part of the durable
  workflow contract.

A workflow author can manually model a compensation as another ordinary state today. That does not
provide AC/DC. The platform would not know that this state is an inverse obligation, would not
automatically schedule it in reverse order, would not make its execution mandatory after a later
failure, and would not know whether it restores correctness under concurrency.

## Primary Problem To Attack Now

Define and implement a typed compensation/backout model that is as authoritative as the existing
forward side-effect model.

The model must answer these questions:

1. What does a state promise about compensation?
2. What evidence proves that compensation is required?
3. What evidence proves that compensation ran exactly once?
4. What evidence proves that the compensation produced a correct terminal condition?
5. What happens when compensation itself fails, is ambiguous, or requires human input?
6. How does certification prevent side-effecting workflows from omitting backout semantics?
7. How does runtime choose between retry, alternate path, compensation, and manual resolution?
8. What concurrency guarantees can MFM actually make, and what must be declared as a domain
   assertion rather than a platform proof?

## Required Semantics

### 1. Failure Directives Are Certified Semantics

Failure handling cannot live in ad hoc app or CLI glue. A certified spec must carry failure
directives for relevant nodes:

- retry with bounded policy;
- take a certified alternate path;
- begin compensation/backout;
- block for manual resolution;
- terminal fail only when no side effects require remediation.

"Do nothing" is not an acceptable directive for a side-effecting workflow because it can leave an
incomplete and inconsistent run.

### 2. Compensation Is A Typed Side Effect

Compensation must be represented as workflow execution, not as mutable cleanup outside the run
stream.

A compensation state needs the same class of durable semantics as forward side effects:

- deterministic compensation intent;
- deterministic compensation idempotency input;
- typed capability contract;
- durable claim and fencing;
- invocation-started uncertainty boundary;
- submission, unknown, not-submitted, receipt, confirmation, ambiguous, and failure evidence;
- replay verifier identity;
- terminal output binding.

The run stream must append compensation events. It must never erase, rewrite, or "undo" forward
events.

### 3. Reverse Order Must Be Platform-Owned

For saga-style compensation, completed forward side effects must be compensated in reverse
dependency order unless the certified spec proves a different safe partial order.

The scheduler, not application code, must derive the compensation frontier from:

- the certified graph;
- completed cells and side-effect ledgers;
- dependency edges;
- declared compensation contracts;
- existing compensation evidence.

This makes compensation crash-resumable and once-and-only-once.

### 4. Correctness Requires More Than Inverses

The platform must not equate "ran an inverse operation" with "restored correctness".

For each side-effecting state, MFM needs a declared compensation correctness class. Examples:

- commutative inverse, such as decrement then increment under declared constraints;
- escrow or bounded counter operation with invariant-preserving compensation;
- predicate update with captured qualifying set;
- external service operation with idempotent cancel/refund/release endpoint;
- irreversible operation allowed only as the final irreversible boundary;
- manual-only remediation.

If a state cannot provide a platform-checkable correctness class, certification should force an
explicit domain assertion or manual-resolution path. The assertion must be visible in the certified
spec and replayable evidence; it must not be hidden in code comments or docs.

### 5. Phantom And Cascading Cases Must Be Explicit

Concurrent workflows create the phantom problem. A compensation that re-runs a predicate later can
touch records that were not touched by the original step, and a compensation that blindly restores an
old value can erase intervening work.

MFM needs typed evidence for the set or semantic scope originally affected by a side effect. For
database-like effects, this may mean predicate snapshot evidence, touched-key evidence, or an
isolation/lock proof. For external systems, this may mean service-provided operation identifiers,
prior/post state evidence, or an explicit ambiguity/manual-resolution record.

If the platform cannot prove the compensation is safe, the run must not be marked compensated.

### 6. Irreversible Effects Shape Workflow Topology

Some effects cannot be undone: dispensing funds, publishing an irreversible on-chain transaction,
notifying an external party without cancellation semantics, or crossing a business finality
boundary.

MFM must make irreversible boundaries explicit. A certified workflow with an irreversible effect must
either:

- place the irreversible effect after all fallible compensatable work;
- split the business process into multiple workflows;
- require a manual-resolution terminal state for failures after the irreversible boundary.

The runtime must not pretend such a workflow can be atomically backed out.

### 7. Manual Resolution Is A Durable State

Manual intervention is not an out-of-band escape hatch. It must be represented by typed events and
publicly inspectable run status.

Manual resolution must record:

- why automatic compensation could not continue;
- what side effects remain unresolved;
- what authority or operator action resolved them;
- what evidence allows the run to become terminal.

This evidence must be redaction-safe and must not persist secrets.

## Architecture Constraints

Any solution must preserve the existing MFM boundary contract:

- operations plan topology and failure directives, but do not execute compensation;
- states declare forward and compensation semantics, but do not create live IO;
- adapters bind forward and compensation intent to capabilities and evidence phases;
- transports implement reusable live/replay capability backends;
- store owns append-only event admission, logical keys, preconditions, side-effect projections, and
  compensation projections;
- runtime owns scheduler decisions, compensation frontier derivation, and guarded commits;
- app assembles registries, stores, artifacts, runners, and capabilities;
- CLI and REST expose start/resume/replay/inspect/render surfaces only.

Compensation cannot be bolted on in binaries, app glue, storage projections alone, or transport
retry code. It must become certified typed execution semantics.

## Non-Goals

The immediate goal is not to implement long-running global locks as the default. Physical backout
can be useful for narrow database-local cases, but MFM workflows commonly involve external services
and on-chain effects where saga-style compensation or explicit manual resolution is the realistic
path.

The immediate goal is not to guarantee serializable isolation for all sagas. The problem is to make
the guarantee explicit, enforce what the platform can prove, and block or require assertions where
correctness depends on domain-specific facts.

The immediate goal is not to erase failed workflow history. Append-only history remains authority;
AC/DC is achieved by appending remediation evidence, not by mutating the past.

## Success Criteria

MFM can start claiming AC/DC progress only when all of the following are true:

- certification rejects side-effecting workflows with no declared failure/backout strategy;
- a failed run with confirmed side effects automatically enters a durable compensation or manual
  resolution mode;
- compensation steps are typed, idempotent, replay-verifiable, and crash-resumable;
- the store projects forward and compensation ledger status from append-only events;
- resume can continue incomplete compensation without duplicating external mutations;
- replay can verify compensation evidence without live capabilities;
- public inspect/render surfaces show unresolved, compensating, compensated, ambiguous, and manual
  resolution states distinctly;
- tests cover failure after one confirmed side effect, failure after multiple side effects,
  compensation crash-resume, ambiguous compensation, irreversible boundary rejection, and at least
  one phantom-prone compensation case that must block or require explicit evidence.

## Open Design Questions

- Should compensation be part of the same certified spec, or should failure instantiate a certified
  continuation spec derived from the original run?
- How should MFM express compensation correctness classes without making kernel crates depend on
  domain semantics?
- What is the minimal event vocabulary for compensation without duplicating the entire forward
  side-effect event model?
- How should alternate paths interact with already completed side effects?
- Which compensation guarantees are framework-enforced, and which are domain assertions carried as
  certified evidence?
- What is the public API contract for runs that are failed, compensating, compensated, ambiguous, or
  manually resolved?
- How should on-chain finality and reorg/replay evidence affect irreversible-boundary classification?

## Immediate Attack Surface

The first implementation design should focus on the smallest vertical slice:

1. Add certified failure directives and compensation metadata to typed program/spec descriptors.
2. Add a compensation contract shape for `SideEffectState` or a parallel trait with typed intent,
   idempotency, receipt, confirmation, and correctness class evidence.
3. Add append-only compensation ledger events and store projections.
4. Teach runtime to enter compensation mode after a non-retryable failure with confirmed forward
   side effects.
5. Add replay/resume verification for compensation evidence.
6. Expose distinct run phases for unresolved failure, compensating, compensated, ambiguous, and
   manual resolution.
7. Add focused tests before adding broad workflow features.

This is the problem MFM should attack now: extend the existing typed durable execution core from
"we can resume the forward workflow safely" to "we can finish, compensate, or durably escalate the
entire workflow with explicit correctness semantics."
