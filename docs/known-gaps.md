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
indefinitely. Only the EVM domain resolves it, by re-observing chain state on a
fresh run. A new domain with external effects inherits the safety property and
strands runs on crash, and nothing in the type system requires it to supply a
re-observation procedure.

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
