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

## No recovery from a possible external entry that cannot absorb

If a process dies between invoking an external effect and committing its
observation, or the invoker reports `EntryUnknown`, the run holds an unresolved
authorization. A capability declaring `EntryAbsorbing<MAX>` resolves both shapes
by closing the attempt and re-asserting the byte-identical committed request.
Three residuals remain, and all three are declared rather than discovered.

`EntryOnce` is the first and it is not a weak answer. Where no key exists — a
Postgres insert with an autoincrement primary key and no natural key, an HTTP
POST against a server with no idempotency support — or where the response is
once-only, so a deduped repeat cannot produce an inhabitant of `Returned`, the
occurrence parks exactly as it always did. The difference is that the gap is now
a certified, per-capability, compile-time-forced property, visible in the program
document and reportable before deployment.

A spent budget is the second. The bound is `MAX_ENTRIES` authorizations per
occurrence, and the last one is never closed, because its closure would authorize
an entry beyond the bound. That attempt is left with a dangling authorization, so
"every attempt carries a terminal observation" holds for resolved histories only.

Windowed absorption is the third and the worst, because it looks resolvable. The
fold has no clock and cannot have one — a wall-clock-conditioned fold breaks
refold equivalence — so absorption backed by a retention window rather than
durable state is indistinguishable, at the contract, from absorption that does
not expire. A run re-asserted past the window is re-entered without absorbing.
Discovery latency is the whole exposure; `list_parked_runs` is what bounds it.

Four things are declared and unverifiable, all asserted by naming
`EntryAbsorbing<MAX>`: that the external system absorbs a repeat, that the
adapter transmits the entry key, that absorption is retained long enough, and
that `Returned` is a function of the external system's post-state rather than of
one exchange. The last is checkable by review only.

See [`effect-entry-resolution.md`](effect-entry-resolution.md) for the mechanism
and for the uniqueness trade it spends.

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

## Verification residuals

Carried from the TT2 review. Each is implemented but lacks the proof its
contract claims.

| Area | Missing evidence |
| --- | --- |
| Canonical ingress | The generated, hostile, and production-scale acceptance matrix. Exact-limit and one-over boundaries are covered per field; the full generated corpus is not. |
| SQL ownership | An independent audit of every production query and its scale behaviour. The AST inventory proves ownership of the current set only. |
| EVM wallet queries | A production latency envelope. Query shape, non-aggregate predicates, indexed plans, and 64-reservation scale are covered. |
| EVM provider trust | Deployment-owned provider trust, and the broader crash, ambiguity, and production-authority matrices. Repository-local crash boundaries at broadcast, receipt, finality, and completion are covered, as is one in-run absorption of an ambiguous broadcast. A process kill *between* an Effect authorization and its observation is covered at the Runtime boundary by call count, not in the two-process production harness, which has no seam inside one drive. |
| EVM fault matrix | Cross-process, replacement, and scale fault matrices. |
| Portable replay | Live application multi-hop export and live Postgres replay. The generated corpus, offline parity, bounds, and tamper cases are covered. |
| Purpose isolation | The complete runtime/audit/replay/export isolation matrix. A 16-case compile-fail matrix and package regressions are covered. |
| Keystore | External termination, OOM, and resource-failure classes. Allocation continuity, cleanup, and authentication/identity failures are covered. |
| Retained-release trust | A concrete production implementation. Portable tests use explicit test implementations. |
| Wallet prefix tampering | A prefix altered while a fenced provider is down is no longer refused at restart. The promotion path still compares the digest the child was configured with, but that digest travels with the configuration and is not an independent record. |
| Ownership | Hand-written production LOC remains concentrated in the store and storage modules; the remediation added roughly 19k net lines outside documentation. |
