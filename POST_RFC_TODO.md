# Post-RFC Follow-up Work

Status: deferred follow-up backlog. None of these items weakens or postpones the current
recoverability contract in `docs/design.md` or `RFC_REFACTOR_RECOVERABILITY.md`.

This file records work discovered after the recoverability refactor closed. An item belongs here
only when the current repository is internally consistent without it and the deferred work can land
later as a complete current-design change. Each implementation must include its tests,
documentation, Nixfied task-graph update, and one or more ordered logical commits. Do not restore a
superseded API or compatibility path to satisfy a follow-up.

## Required Follow-ups

### PRF-001: Restore qualified EVM wallet parity against managed Reth

The deleted `tests/integration/tests/parity_reth_eip1559.rs` exercised a real Reth node through the
superseded estimate/sign/send session. The current wallet conformance suite exercises the new
executor and exact JSON-RPC protocol against a loopback server, but it does not prove compatibility
with a real Reth implementation.

Implement a new parity target at the current boundary:

- restore a pinned managed Reth service through Nixfied without restoring the old EVM session;
- execute the qualified wallet executor with its exact route, chain, signer, account-sequence,
  generation, fence, and assurance bindings;
- submit a type-2 transaction through `eth_sendRawTransaction`;
- recover it through exact-hash transaction and receipt reads;
- verify sender, nonce, chain, destination, value, calldata, access list, gas limit, fee fields,
  transaction hash, receipt, finalized head, and canonical inclusion;
- prove the signed envelope, signature, private key, unlock input, endpoint, and authorization never
  enter retained evidence, logs, errors, or public output;
- retain current negative and restart coverage in the loopback suite rather than duplicating it in
  the real-node parity test; and
- add the parity leaf to `.#ci` after database qualification and before the closing source revision.

The new test must not restore gas-estimation ownership, `EvmTransactionSession`, standalone
transaction signing, or raw signed-transaction file output.

### PRF-002: Restore end-to-end REST and PostgreSQL parity at the current lifecycle

The deleted `tests/integration/tests/parity_rest_api_postgres_smoke.rs` exercised the superseded
REST lifecycle and application composition. Current REST contract tests and PostgreSQL conformance
tests cover their boundaries independently, but there is no single test proving that the current
production application, router, and authoritative PostgreSQL stores compose correctly.

Implement a new integration target that:

- provisions the run-journal and executor schemas with distinct roles and independent
  writer-generation fences;
- constructs the production application through the same qualified composition used by a
  deployment host, with no test-only unfenced fallback;
- proves readiness checks both stores and both fences;
- discovers exactly `mfm.portfolio/snapshot@1` and `mfm.evm/submit-transaction@1`;
- authenticates and authorizes the current admit, drive, public-read, replay, trace, audit, and
  export surfaces through the REST router;
- completes at least one deterministic run, restarts the application over the same stores, and
  proves resume and replay perform no repeated live operation after retained evidence is committed;
- checks reviewed error parity for invalid JSON, unknown entry point, absent or cross-tenant run,
  stale fence, and unavailable readiness; and
- runs under the managed PostgreSQL lane in `.#test-db` and therefore in `.#ci`.

The test must not restore old start/resume/status routes, implicit run-to-completion behavior,
configured-target compatibility DTOs, replica authority, or a standalone binary-owned fence.

### PRF-003: Make authorized effect completion recording exhaustive

Treat the run journal as the authoritative append-only transaction history of a run, not as an
optional subsystem that callers notify after work succeeds. Runtime must internalize the recording
protocol so that every related step is durably connected by exact references:

```text
EffectRequested
  -> ExternalAccessAuthorized
  -> one affine executor invocation
  -> ExternalAccessObserved(Returned | DidNotEnter | Indeterminate)
  -> EffectSettled
```

The current closed `EffectExecutorOutcome::DidNotEnter` and
`EffectExecutorOutcome::Indeterminate` paths are recorded correctly. The remaining escape hatch is
`RecoverableEffectExecutor::ensure` returning
`Result<EffectExecutorOutcome, ExecutorError>`: an outer `ExecutorError` can survive after runtime
has committed and consumed the authorization while bypassing `ExternalAccessObserved`.

Replace that split completion contract with one exhaustive boundary:

- failures detected before authorization remain ordinary runtime rejection and create no live
  authority;
- after authorization is handed to the executor, every surviving completion is converted into one
  closed, redaction-safe, authorization-linked observation and durably appended before runtime
  returns control;
- a proven pre-entry failure records `DidNotEnter`;
- a failure after entry may have occurred records `Indeterminate`;
- integrity or contract failures remain audit-only and blocking unless exact reviewed evidence
  permits semantic consumption; they must never be disguised as a successful effect or an ordinary
  domain failure;
- process death or panic before a result survives leaves the unmatched authorization as
  `CrashAmbiguous`; runtime must not invent an observation;
- the later settlement transition consumes only an eligible exact observation reference; and
- replay derives the same pending, blocked, or settled state exclusively by folding the committed
  history.

Keep authorization and observation as separate atomic appends. Runtime must not hold a database
transaction across signer, destination, wallet, or network IO. Keep the executor's keyed
delivery/resource ledger as an independent authority and failure domain; the run history records
its interaction and exact retained-evidence references rather than duplicating its internal
transitions. `mfm-journal` may remain a domain-free schema and codec crate, but no journal API or
crate boundary may make authoritative recording optional to runtime execution.

Add boundary-focused verification that:

- exhaustively classifies every executor error by whether authorization was consumed and whether
  boundary entry can be excluded;
- injects each surviving failure class after authorization and proves exactly one linked
  observation is committed;
- proves audit-only integrity outcomes cannot settle a state;
- distinguishes a surviving failure from process loss, retaining an unmatched authorization only
  for the latter;
- resumes after each authorization, invocation, observation, and settlement crash boundary without
  repeating unauthorized IO; and
- replays the resulting histories without executor, adapter, state, or classifier callbacks.

## Explicit Non-Goals

- Do not restore `mfm keystore tx-sign`.
- Do not restore the standalone app transaction-signing service.
- Do not persist or publish raw signed transaction bytes.
- Do not restore the old EVM estimate/sign/send session.
- Do not restore the old REST lifecycle or application bootstrap.
- Do not collapse authorization, external IO, and observation into one database transaction.
- Do not absorb executor delivery or resource-policy history into the run history.
- Do not count mock, loopback, unit, or compile-fail coverage as real-service parity, even when that
  coverage remains necessary.

## Findings Under Review

Add further findings here only after confirming the current owner, missing guarantee, existing
coverage, and exact acceptance boundary. Promote a finding to `Required Follow-ups` once it is
specific enough to implement and verify without inventing a parallel design.

## Material Uncertainties

- Managed Reth finality behavior for the pinned development topology has not yet been revalidated
  against the wallet executor's exact finalized-head contract. If the node cannot expose the
  required finalized tag and canonical inclusion semantics, the parity topology must be changed;
  weakening the wallet assurance contract or accepting receipt-only success is not allowed.
- The current `ExecutorError` taxonomy mixes pre-entry contract rejection, integrity corruption,
  durable-backend failure, append ambiguity, and injected process loss. Before PRF-003 is
  implemented, every variant must be assigned an exact authority-consumption and boundary-entry
  classification. A wrong classification could either leave a surviving completion unaudited or
  falsely claim that an external effect did not occur.
- It is not yet established whether the existing `DidNotEnter` and `Indeterminate` observation
  variants can honestly represent every surviving audit-only integrity failure. Reusing them may
  conflate effect uncertainty with local corruption; adding a non-consumable observation outcome
  changes the frozen recoverability schema. Resolve this in one target design before changing the
  executor or journal APIs.
