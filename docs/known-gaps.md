# Known gaps

Properties this repository does not currently guarantee, and verification the
contracts in [`design.md`](design.md) and [`architecture.md`](architecture.md)
assume but do not yet demonstrate. Each entry states what is missing, not what
is planned.

This file replaces the TT1/TT2 problem ledgers, implementation plans, and
review documents. Those recorded a remediation that is complete; what survives
of them is below.

## Absent append-only witness

PostgreSQL is the sole authority for append-only history, in both the
structured run/configuration stores and the EVM wallet authority. Nothing
retains an independent record of a stream's head outside the database.

A rolled-back database is a valid earlier state of itself, so no check against
its own contents can detect a restore. If a deployment rewinds committed
history, MFM accepts the rewound state and appends onto it, and the result
verifies internally. Detecting that is a deployment responsibility outside this
repository.

Any future witness must be the one component with no restore path — monotonic,
never restored from backup, fail-closed on loss. A witness restored alongside
the database it witnesses proves nothing. The previous external checkpoint
authority did not state that requirement and had no production implementation,
which is why it was removed rather than kept as a partial guarantee.

See *Append-only authority and the absent witness* in [`design.md`](design.md).

## No generic recovery from a possible external entry

If a process dies between invoking an external effect and committing its
observation, the run holds a committed authorization with no observation.

The safety half is generic and enforced in the store fold: the effect ran only
after its authorization was durable, a second authorization for the same
occurrence is rejected, and the state is never actionable, so no duplicate
effect is possible in any domain.

Recovery is not generic. `DriveOutcome::PossibleEntry` is terminal in the
runtime and surfaces as an operational block in `mfm-app`; the run parks
indefinitely. A new domain with external effects inherits the safety property
and strands runs on crash, and nothing in the type system requires it to declare
whether recovery is possible at all.

No domain resolves a parked run, including EVM. What EVM recovers is the
submission *intent*, and the caller does it: `derive_submission_intent_id`
hashes only the domain, issuer, and caller submission token, so a fresh run
under the same token re-derives the same reservation and operation keys, finds
the retained state, and observes the chain instead of broadcasting again. The
nonce reservation is what makes that cross-run retry safe. The originally parked
run stays parked forever, and every in-run reconciliation branch is settlement
of a committed observation, so none of them is reachable from a park.

The fold reaches the terminal frontier without consulting any per-capability
property. A capability whose external system would absorb a repeat of the exact
committed request is folded identically to one that would duplicate, because
there is nothing for either to declare.

The EVM resolution is not portable as written, and neither is the axis that
makes it work. `Refreshable<E>` with `RefreshableBinding<Resource>` names an
exclusive resource lineage and a non-entry evidence type, and the store verifies
that negative callback-free against the retained release history — but declaring
it requires a `RuntimeResourceAuthority`, a resource lineage contract, a public
lineage head, and store-side supersession verification. A table with a unique
index has none of those and needs none of them.

Two things are also absent that any recovery mechanism would need. The fold
computes the full identity of the parked occurrence and then discards it:
`StructuredFrontier::PossibleEntry` and `DriveOutcome::PossibleEntry` are unit
variants, so the operator is told that a run is blocked and not which occurrence
blocked it, though the access-audit projection retains every field. And the run
cannot be found at all without its run id; see *No way to find a parked run*.

See [`effect-entry-resolution.md`](effect-entry-resolution.md) for the proposed
per-capability contract, which declares absorption rather than a slot. It is a
proposal whose central trade — replacing structural one-entry-per-occurrence
with a bounded, declared maximum — is not yet accepted. It is not an implemented
guarantee.

## No way to find a parked run

Every application entry point is per-`RunId`, and there is no listing surface.
An operator who does not already hold the run id of a parked run cannot
discover it, so parks are found late or by accident.

This is independent of how a park is resolved. Every recovery design assumes a
caller who already knows which run to fix, and nothing produces that caller.

It is also load-bearing for any absorption-based recovery, because discovery
latency is the whole exposure where absorption is backed by a retention window
rather than durable state: a park found in minutes re-asserts inside any
plausible window, and a park found weeks later re-asserts outside it and
duplicates.

## A crashed Read attempt strands its run

The same crash boundary strands a `Read` occurrence. An authorization with no
observation folds to `WaitingReads`, and the fold's per-occurrence
single-outstanding-attempt rule forbids a second attempt, so nothing can
discharge it. `drive_once` reports the frontier without acting and the run never
progresses. The public projection labels it `retryable_evidence_gap`; nothing
retries it.

Unlike the Effect case there is no safety obstacle. A Read is already defined as
not consuming externally meaningful state, so a second attempt is sound by
construction. The restriction is uniform rather than required.

That definition is prose, not a certification obligation. Nothing checks that a
capability declared `Read` consumes nothing, and any recovery that reissues a
Read makes the definition load-bearing where it currently is not.

`fresh_folds_resume_every_runtime_crash_boundary` in `mfm-store` covers this
boundary only because the test still holds the in-process authorization handle
that a real crash destroys.

[`effect-entry-resolution.md`](effect-entry-resolution.md) resolves this gap
with the same leaf it proposes for the Effect case; a Read needs no
declaration because a repeat absorbs vacuously.

## Verification residuals

Carried from the TT2 review. Each is implemented but lacks the proof its
contract claims.

| Area | Missing evidence |
| --- | --- |
| Canonical ingress | The generated, hostile, and production-scale acceptance matrix. Exact-limit and one-over boundaries are covered per field; the full generated corpus is not. |
| SQL ownership | An independent audit of every production query and its scale behaviour. The AST inventory proves ownership of the current set only. |
| EVM wallet queries | A production latency envelope. Query shape, non-aggregate predicates, indexed plans, and 64-reservation scale are covered. |
| EVM provider trust | Deployment-owned provider trust, and the broader crash, ambiguity, and production-authority matrices. Repository-local crash boundaries at broadcast, receipt, finality, and completion are covered. |
| EVM fault matrix | Cross-process, replacement, and scale fault matrices. |
| Portable replay | Live application multi-hop export and live Postgres replay. The generated corpus, offline parity, bounds, and tamper cases are covered. |
| Purpose isolation | The complete runtime/audit/replay/export isolation matrix. A 16-case compile-fail matrix and package regressions are covered. |
| Keystore | External termination, OOM, and resource-failure classes. Allocation continuity, cleanup, and authentication/identity failures are covered. |
| Retained-release trust | A concrete production implementation. Portable tests use explicit test implementations. |
| Wallet prefix tampering | A prefix altered while a fenced provider is down is no longer refused at restart. The promotion path still compares the digest the child was configured with, but that digest travels with the configuration and is not an independent record. |
| Ownership | Hand-written production LOC remains concentrated in the store and storage modules; the remediation added roughly 19k net lines outside documentation. |
