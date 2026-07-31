# Implementation Plan: Runtime History Choke Point

Status: ready for implementation

Branch: `refact-runtime`

Starting point: `3fb221c5c0a4129c6d34f9b55425b1b17c1e253d`

Target design: [`RFC_RUNTIME_HISTORY_CHOKE_POINT.md`](RFC_RUNTIME_HISTORY_CHOKE_POINT.md)

`docs/design.md` remains the repository's current design contract until the cutover commit updates
the implementation, persisted contracts, tests, and design documentation together.

## Material uncertainties

none

The deployment fence boundary is deliberately resolved at the repository edge: this repository
implements the sealed client/enforcement port and its conformance tests, while qualified deployment
infrastructure supplies the production issuer, target-held key, revocation, and promotion control
plane. Production assembly must fail closed when that provider is absent; no in-repository fallback
or self-attestation is allowed. Qualification exercises the boundary through a distinct process
and live connection; public request/reply data is evidence only and can never itself construct
authority.

## Goal

Replace the arbitrary state graph and generic executor with one declaration-ordered structured
program, one callback-free history fold, and one Runtime cursor interpreter. Keep EVM and nonce
semantics out of the generic kernel by expressing EVM progression as injected states backed by a
separate PostgreSQL wallet-nonce authority.

The finished tree must have one current design. Do not add graph lowering, compatibility readers,
dual schemas, a renamed executor, an in-memory PostgreSQL substitute, or a second runtime/store
fold.

## Commit Sequence

Create these commits on this branch after the plan update:

1. `replace graph and executor with structured runtime history`
2. `harden structured runtime and wallet authority qualification`

The first commit is the complete code, schema, application, documentation, and deletion cutover.
The second adds adversarial, crash, and concurrency coverage plus small fixes exposed by that
coverage. If the second commit discovers a missing production type, protocol, state, schema, or
application path, amend the first commit instead of hiding implementation work in hardening.

Engineers may use local work-in-progress commits, but the final history should contain the two
logical commits above. No committed intermediate state may retain both graph/executor and
structured-runtime paths.

## Responsibility Placement

| Owner | Responsibility |
| --- | --- |
| `mfm-program` and `mfm-program-derive` | Ordered `State`, exhaustive `Match`, bounded `FanOut`, child composition, lexical values, and typed block/root results. |
| `mfm-spec` and `mfm-certify` | Canonical structured programs, pure expansion, structural/type validation, failure plans, fan-out bounds, policy coverage, and the sole `CertifiedProgram` authority. |
| `mfm-journal` | Exactly the five run-history record families. |
| `mfm-store` | Exact-head candidate validation, object/fact/event closure, the sole callback-free fold, verified cursor, and memory conformance. |
| `mfm-storage-postgres` | EVM-neutral durable persistence of the `mfm-store` contract and its authoritative writer fence. |
| `mfm-runtime` | Sole `RunHistoryWriter`, admission, one cursor action, callback invocation, and the private Read/Effect access bracket. |
| `mfm-replay` | Recorded verification and projections over the store fold, with no live IO or second reducer. |
| `mfm-evm` | EVM contracts, states, structured expansions, wallet-domain semantics, and wallet-nonce port types. |
| `mfm-evm-live` | Direct Runtime-authorized EVM adapters over reusable transports and signers; no scheduler or lifecycle. |
| `mfm-storage-evm-postgres` | Real PostgreSQL activation-registry and wallet-nonce authority implementations, with private pools and sealed target-bound access. |
| Qualified deployment infrastructure | Chain-instance issuance, external writer fencing, target/session qualification, sender-path fencing, and promotion orchestration. |
| `mfm-app`, CLI, and REST | Qualification and assembly, `admit_run`/`drive_once`, purpose-limited reads, and reviewed output rendering only. |

## Commit 1 — Replace Graph And Executor With Structured Runtime History

Subject: `replace graph and executor with structured runtime history`

This is one breaking cutover. The sections below are its internal implementation order.

### 1. Replace the program model

- Make `State`, `Match`, and `FanOut` the only author-visible structural forms. An authored child
  operation call is expansion sugar and must not survive in the expanded program.
- Make only `State` executable. Matches, fan-out groups, lanes, fragments, joins, and outcomes do
  not receive executable occurrence identities or history transitions.
- Use declaration order as the normal execution order. Replace arbitrary graph edges with typed
  lexical handles, exact producer references, branch merges, fragment inputs, and fan-out joins.
- Represent block results as typed lexical tail expressions. Root helpers construct
  `OperationOutcome::Success` or `OperationOutcome::Failure`; they are not instructions.
- Keep state, lane, fragment, and operation outcomes nominally distinct so a value from one
  structural role cannot be substituted for another.
- Implement `FailureContract::{Never, Typed}`. `Never` has no value schema, codec, producer, or
  retained slot. Every `Typed` state or fragment boundary has exactly one certified failure plan.
- Implement explicit recovery and lexical default failure handling as ordinary injected states.
  A default handler is `Pure`, infallible, and selected by the exact source failure contract. A
  `Never` scope has no default and must recover every fallible child explicitly.
- Make `Match` exhaustive over the canonical tag of a committed closed sum. The caller never
  writes a branch choice.
- Make `FanOut` non-empty, collect-all, declaration ordered, and limited transitively to `Pure`
  and `Read`. Preserve the certified depth-two case used by portfolio plus child EVM reads; reject
  deeper nesting and every Effect lane.

### 2. Replace expansion and certification

- Use one pure pipeline:

  ```text
  child substitution
    -> capability lowering
    -> policy wrapping
    -> failure completion
    -> normalization and certification
  ```

- Keep expansion deterministic, finite, bounded, call-site-local, and free of ambient IO.
  Injected support states are visible ordinary leaves and do not recursively expand.
- Allow expansion to inject pre/post states, failure handlers, security checks, provenance, and
  EVM support without adding special Runtime behavior.
- Certify declaration order, lexical dominance, branch exhaustiveness, exact failure plans,
  protected fragment boundaries, fan-out restrictions, capability bindings, implementation
  manifests, policy coverage, and one total root outcome.
- Admit one canonical content-addressed `CertifiedProgram` closure binding the authored and
  expanded programs, expansion profile and proof, policy and predicate set, contracts, bounds,
  and implementation manifests. Resolve the trusted predicate/profile policy from the qualified
  entry-point registry; callers cannot select weaker rules.
- Delete graph cycle, reachability, alternative-source, dependency-skip, required-success,
  terminal-node, ready-set, and graph-wide phase certification.

### 3. Replace run history and storage

- Keep exactly these append-only record families:

  ```text
  RunAdmitted
  StateTransitionCommitted
  ExternalAccessAuthorized
  ExternalAccessObserved
  RunClosed
  ```

- Implement one callback-free fold in `mfm-store`. It verifies the complete prefix and derives
  lexical bindings, the structured cursor, active fan-out lanes, outstanding access, semantic and
  journal heads, integrity state, and the root outcome.
- Make the store the final authority for hostile persisted input: canonical encoding and hashes,
  exact predecessor/head, logical-key uniqueness, certified-program closure, provenance,
  cursor/action legality, access linkage, object/fact closure, and terminal closure.
- Append each candidate atomically with all new objects, facts, and records. An exact-head race
  commits the whole candidate or none of it.
- Append `RunClosed` in the same atomic candidate that first makes the root outcome derivable,
  including the zero-state `RunAdmitted + RunClosed` case.
- Store no mutable cursor row and no control record for Match, FanOut, joins, fragments, or result
  expressions.
- Implement identical conformance semantics for the memory backend and the real
  `mfm-storage-postgres` backend. PostgreSQL types must execute SQL; they must never delegate to an
  in-memory map.
- Retain a deployment-supplied `AuthoritativeWriterFence` (or its direct replacement) for the
  EVM-neutral RunHistory store. Its non-rollback writer lineage and stale/sibling-writer exclusion
  remain separate from the wallet fence and remain part of production readiness.

### 4. Replace Runtime and replay

- Make Runtime a cursor interpreter over the verified fold, not a global graph scheduler.
  Outside fan-out there is one current state. Inside fan-out, select the minimum
  declaration-ordered actionable lane path; a waiting Read may expose a later lane, while an
  unresolved barrier stops the scan. Joined results always remain in declaration order.
- Keep `drive_once` bounded to one semantic transition or one audited external-access operation.
- Make Runtime the only production holder of the non-cloneable `RunHistoryWriter`.
- Seal the access path inside Runtime:

  ```text
  Prepared<K>
    -> committed authorization
    -> Authorized<K>
    -> one registered invoker entry
    -> PendingObservation<K>
    -> committed observation
    -> CommittedObservation<K>
  ```

- A committed authorization may create exactly one affine invocation authority. The registered
  invoker receives the frozen request, performs its one bounded operation, and has no outer normal
  error path after accepting authority.
- Persist every normal invoker completion before it can reach a state or successful drive result.
  A stale-head observation retry may rebuild only the append envelope; it must not invoke again.
  Resolve an ambiguous acknowledgement for the original append identity before rebasing.
- Replace the current access completion algebra explicitly:

  ```text
  Read   = Returned | SafeFailure | IntegrityFault
  Effect = Returned | SafeFailure | SupersededBeforeEntry | EntryUnknown | IntegrityFault
  ```

  Delete `DidNotEnter`, `Indeterminate`, `NonDomainFailure`, the public `non_domain_failure`
  projection, and every post-closure audit-tail rule. No record is legal after `RunClosed`.
- Make overlapping same-occurrence attempts unrepresentable. Only committed
  `SupersededBeforeEntry` may produce the next Effect attempt. An unmatched Read remains waiting;
  an Effect with possible entry remains parked; committed integrity evidence blocks.
- Classify resource rotation exactly: a stale or revoked Effect authority proved before entry may
  return only `SupersededBeforeEntry`; a stale Read returns its reviewed `SafeFailure`; and any
  Effect for which non-entry cannot be proved returns `EntryUnknown` and remains parked.
- Commit the exact result produced by the registered state callback. Runtime must not synthesize
  output, failure, facts, or terminal meaning.
- Route definite `Returned` and reviewed `SafeFailure` evidence through the state's exact typed
  handler. An ordinary failed run closes normally and does not block a later distinct `run_id`.
- Replace the special fact-selection execution kind with an ordinary authorized `Read` backed by
  the retained purpose-limited scanner and frontier proof.
- Give the store only a purpose-limited public physical-binding certificate/lineage verifier. The
  store uses it to author the authorization's public binding reference and reject rollback or
  sibling refresh heads; it never receives a live target session, mutation permit, fence issuer,
  or nonce authority.
- Make replay, trace, export, CLI, and REST projections use the same callback-free fold. Replay
  performs no callback, provider, signer, nonce-authority, or other live IO.

### 5. Rewrite operations and EVM execution

- Rewrite all three production operations in the structured DSL:

  - `PortfolioSnapshotOperation`
  - `EvmBalanceCollectionOperation`
  - `EvmSubmitTransactionOperation`

- Preserve portfolio fan-out containing each child EVM read fan-out at certified depth two.
- Express every condition as `Match`, and every retry, poll, replacement, or recovery step as an
  explicit bounded state occurrence with its own failure handling.
- Define `WalletNonceDomain` as qualified physical chain instance plus sender. Verify a
  deployment-issued immutable chain declaration containing chain ID, genesis, a never-reused
  instance namespace, and a finalized fork anchor; every admitted route must prove membership.
  Redundant routes may share one declaration, but cloned or independently operated forks may not.
- Replace executor ensure/settle behavior with direct Runtime-authorized Read/Effect invokers and
  one registered EVM submission expansion. That expansion injects domain/intent derivation,
  status reads, pending observation, reserve, candidate construction, signer attestation,
  activation, exact broadcast, receipt/finality observation, reconciliation, completion, and
  projection states.
- Require every reservation attempt, including first use and later calls, to consume a fresh
  committed `eth_getTransactionCount(sender, "pending")` observation. First use requires equality
  with the qualified finalized nonce floor. Later use allocates `local_high_water + 1` only when
  pending is equal or behind; provider-ahead or first-use mismatch returns typed divergence
  without mutation.
- Keep stable intent, reservation, candidate, and completion identities independent of `run_id`
  and physical generations. Derive `SubmissionIntentId` from authenticated issuer identity plus a
  bounded caller token; never accept a caller-supplied ID directly. Permit one incomplete intent
  per wallet domain; the same intent resolves permanent progress across runs, while another
  returns typed busy without allocating a nonce.
- Persist a bounded mutation-equivalent candidate family, a contiguous activated prefix, and one
  canonical terminal completion. A later run must be able to observe and finish work started by an
  earlier parked run.
- Attest and broadcast the exact deterministic candidate without retaining private keys,
  signatures, or signed bearer bytes in programs, history, facts, outputs, logs, or errors.
- Keep Runtime, journal, generic store, and replay completely unaware of EVM, wallets, nonces,
  candidates, and transaction lifecycle.

Freeze the authored submission failure contract before generating schemas:

```text
EvmSubmissionFailure =
    TransportUnavailable
  | ProviderUnavailable
  | SignerUnavailable
  | NonceAuthorityUnavailable
  | DestinationRejected
  | ObservationPolicyExhausted
  | ReplacementPolicyExhausted
  | NonceDomainBusy
  | NonceLineageDiverged
  | NonceCapacityExhausted
  | ExecutionReverted
```

Expansion owns the exact leaf-failure mapping into this sum. Possible entry,
`SupersededBeforeEntry`, malformed evidence, and integrity faults remain generic parked/blocked
outcomes and cannot be converted into domain failure.

### 6. Add the PostgreSQL wallet authority

Add one production crate:

```text
crates/storages/evm-postgres/
package: mfm-storage-evm-postgres
```

- Put canonical requests, responses, identities, and state contracts in `mfm-evm`; put SQL,
  transactions, roles, and private pools only in `mfm-storage-evm-postgres`.
- Use distinct activation-registry admin/public, nonce-application, run-history, owner, and test
  roles. No raw or generic PostgreSQL pool escapes assembly, and the authorities cannot
  cross-write.
- Implement permanent wallet-domain activation and one current store-incarnation head per store
  lineage. Domain issuance atomically creates or validates the lineage head and inserts the domain
  binding under the same lineage lock; exact replay returns the original composite proof.
- Require initial activation to prove that old replay/retry ingress and every sender path are
  fenced, all prior effects and resource allocations are terminal, no unresolved possible entry
  remains, and the finalized sender-nonce floor is qualified. If any proof is unavailable, use a
  new sender/domain rather than importing high water or guessing.
- Keep normal status/reserve/activate/complete independent of registry availability. They use an
  immutable activation proof plus offline verification and perform zero registry queries.
- Gate every status snapshot through the sealed current target session. Gate each mutation with a
  fresh non-cloneable, non-serializable permit bound to the exact physical target, database
  session, transaction, store lineage, and writer epoch.
- Resolve a permanent semantic operation key before mutation, lock and revalidate the current
  incarnation, resolve the key again, then apply the mutation and result proof atomically.
- Implement linearizable status, first/later pending allocation, one-incomplete-intent exclusion,
  idempotent reservation, contiguous candidate activation, older-candidate completion, canonical
  result closure, and acknowledgement-ambiguity resolution.
- Consume the deployment-owned external writer fence; do not replace it with a caller-provided
  token, `Box<()>`, static signature, in-process assertion, or public proof. Qualified deployment
  infrastructure owns target/session issuance and end-to-end promotion.
- Promotion order is: irrevocably fence and drain the old target and all sender paths; capture and
  verify the complete final prefix; hydrate and verify the still-closed replacement; publish the
  next incarnation by exact-head registry CAS; then open the replacement. If any proof is missing,
  fail closed and require a new sender/domain.
- Reject old schema bytes. A same-sender incompatible schema or issuer namespace has no migration
  or import path and requires a new sender/domain.

### 7. Cut over application, persisted contracts, and documentation

- Assemble Runtime with the sole run-history writer, qualified registered invokers, and a sealed
  wallet-nonce adapter. Normal application/CLI/REST paths receive no registry admin, fence issuer,
  raw pool, signer secret, or generic invoker authority.
- Keep deployment issuance and promotion behind a separate maintenance boundary.
- Make production `admit_run` and `drive_once` use durable configured history. A parked or failed
  run must not block an unrelated run ID, and a fresh process must continue or read the same open
  or closed history.
- Keep CLI and REST as decode/admit-or-drive/read/render wrappers and preserve their current
  non-interactive, JSON/text, status, and redacted-error behavior unless the new public contract
  deliberately changes it.
- Retain `Application::{entry_points, check_ready, admit_run, drive_once, read_public_run,
  replay_run, read_transition_trace, read_access_audit, export_run}`. Preserve
  `mfm.portfolio/snapshot@1` and `mfm.evm/submit-transaction@1` while replacing their internal
  programs.
- Retain `RunAccessPolicy`, grants, credential handling, tenant isolation, invocation/run identity,
  and per-purpose read/trace/audit/export authorization. Preserve CLI operation listing,
  admit/drive/show/replay/trace/audit/export and keystore commands, plus the current REST health,
  readiness, entry-point, admit, read, drive, replay, trace, audit, and export routes. Do not add
  start/resume, list/watch, fact, manual-resolution, or generic-object endpoints.
- Reset the one current recoverability annex, corpus, schemas, PostgreSQL migration, fixtures, and
  projections in place. Reject all old bytes; do not add a `v2`, compatibility decoder, or dual
  reader/writer.
- Update `docs/design.md`, `docs/architecture.md`, `docs/run-execution.md`,
  `docs/evm-transactions.md`, recoverability/qualification docs, affected crate READMEs,
  `docs/build-and-verification.md`, CLI/REST READMEs, examples, and generated contracts.
- Update `POST_RFC_TODO.md`: retain the real-Reth and REST/PostgreSQL parity work unless this commit
  actually implements it, remove executor terminology, and remove `PRF-003` once its replacement
  tests pass.

### 8. Required tests in commit 1

- DSL unit and compile-fail tests for declaration order, lexical scope, exhaustive Match, FanOut
  restrictions, nominal outcomes, `Never`, missing/duplicate handlers, protected failure plans,
  and forged producer/certification authority.
- Golden expansion/certification tests for all three production operations, including exact
  injected state order, stable bytes/hashes, no surviving child calls, and complete profile/policy
  coverage.
- Shared memory/PostgreSQL conformance for the five record families, exact-head races,
  object/fact/event atomicity, cursor derivation, root closure, callback-free replay, and the
  generic writer fence. PostgreSQL coverage must write in one process/pool, then read and continue
  through a fresh process/pool; an unavailable database must fail without an in-memory fallback.
- Runtime tests proving exact callback inputs/outputs/call counts, one affine invocation, no
  reinvocation during observation persistence, parked ambiguous access, Effect-only refresh, and
  ordinary failure followed by successful execution of another run ID.
- Real PostgreSQL wallet tests for chain-instance/route identity, activation prerequisites,
  issuer-token intent derivation, roles, first/later pending rules, same/different intent races,
  permanent-key idempotency, candidate progression, completion, and status. At least one test must
  write through one process/pool, exit, and read/continue through a fresh process/pool so an
  in-memory substitute cannot pass.
- Core fence tests proving a copied database/public proof cannot read or mutate from another
  target, stale sessions and permits fail, and the replacement cannot open before the old target
  is fenced and its complete prefix is admitted. A test-only fence fixture must not be reachable
  from production assembly.
- Application integration tests running all three operations through production assembly,
  process restart and continued drive/read, independent run IDs, and current CLI/REST projections.
- Old-byte rejection and consuming-crate compile-fail tests for every non-forgeable writer,
  certified-program, access, physical-session, and mutation-permit authority.

## Required Deletions In Commit 1

Delete these crates completely:

- `crates/kernel/executor/`
- `crates/storages/executor-file/`
- `crates/storages/executor-postgres/`

Delete these graph/executor files rather than adapting them:

- `crates/kernel/certify/src/planner.rs`
- `crates/kernel/certify/src/terminal.rs`
- `crates/kernel/runtime/src/decision.rs`
- `crates/kernel/runtime/src/materialization.rs`
- `crates/kernel/spec/src/certified_node.rs`
- `crates/kernel/spec/src/certified_node_tests.rs`
- `crates/live/evm/src/wallet_executor.rs`
- `crates/live/evm/src/wallet_executor_tests.rs`
- `crates/domains/evm/src/wallet_state.rs`
- `crates/app/tests/ui/pass/evm_wallet_executor_uses_qualified_transport.rs`

Replace `tests/integration/tests/evm_audited_graph.rs` with a structured-runtime integration test.

Remove executor lifecycle and next-plan selection from `crates/domains/evm/src/wallet.rs`,
`crates/live/evm/src/wallet_qualification.rs`, and `crates/live/evm/src/wallet_rpc.rs`. Retain only
real domain semantics or reusable bounded transport code; do not preserve old module exports.

Also remove:

- all workspace members, manifest edges, features, lockfile entries, READMEs, migrations, SQLx
  metadata, roles, fixtures, and tests belonging to the deleted crates;
- executor schemas, ledgers, frontiers, tombstones, target authorities, pool/fence/readiness/config
  fields, app errors, CLI/REST fields, trace fields, and recoverability predicates;
- `executor-postgres-qualification` and every executor-specific Nix task or gate edge;
- graph-only nodes, source selectors, dependencies, required-success sets, ready queues, global
  phase maps, and `DependencySkipped`/`BlockingSource` behavior;
- `EffectRequested`, `EffectSettled`, `DidNotEnter`, `Indeterminate`, `NonDomainFailure`, public
  `non_domain_failure` DTO/trace fields, and post-`RunClosed` audit tails;
- the special `FactSelection` execution/access kind after moving its scanner behind ordinary Read;
- the empty or structural `EvmSubmitTransactionFailure` contract after replacing it with
  `EvmSubmissionFailure`; and
- every retired-schema reader, decoder, migration, certifier, compatibility alias, fallback, and
  stale current-design document.

Retain and reuse the canonical/JCS hashing code, value contracts, content-addressed objects and
facts, purpose-limited fact scanner/frontier proof, the five-family journal crate, generic store
primitives, bounded EVM JSON-RPC transport, signer/keystore code, Bitcoin support and parity, app
shell, and CLI/REST transport behavior. Simplification must not delete required validation,
security controls, tests, or documentation.

The mandatory paths remove roughly 27,000 tracked lines. The final production Rust/SQL diff must
remain net-negative without deleting tests, validation, security controls, or necessary docs.

## Commit 2 — Harden Structured Runtime And Wallet Authority Qualification

Subject: `harden structured runtime and wallet authority qualification`

Use the RFC's `Implementation Acceptance Verification` section as the detailed checklist. Add
focused hostile tests in these groups:

- structured-program substitution, wrong-contract/provenance, expansion-order, bound, and
  non-forgeability attacks;
- every Match and depth-two FanOut completion order, stale worker, exact-head race, closure race,
  and callback-free replay mutation;
- crashes and acknowledgement ambiguity at authorization, invocation, observation, settlement,
  and final closure boundaries;
- real PostgreSQL restart, cross-process contention, transaction rollback, role isolation, schema
  mutation denial, unavailable database, and no in-memory fallback;
- nonce first-use/later-use races, lost acknowledgements, competing intents, candidate activation
  races, older-candidate completion, torn-status rejection, and exact terminal convergence;
- copied-target, stale-session, replayed-permit, sibling-writer, promotion crash-point, restore,
  complete-prefix, and sender-path fencing qualification;
- deterministic signer reproduction, exact single submission, bearer non-retention, secret and
  provider-text redaction, and best-effort telemetry failure; and
- process restart with continued drive/read, unrelated parked/closed runs, all three production
  operations, and reviewed CLI/REST output.

This commit may contain narrow fixes revealed by those tests, but it must not add a missing
production architecture path. Delete test helpers that duplicate production semantics; external
systems may be replaced only at explicit transport/provider boundaries.

## Verification And Task Changes

Follow `docs/build-and-verification.md`: use focused package/test commands in the default Nix
development shell while iterating, and expand verification according to the affected boundary.

Required task changes:

- extend `postgres-sqlx-offline-check` to cover `mfm-storage-evm-postgres`;
- extend `postgres-sqlx-check` to validate both current PostgreSQL schemas and query metadata;
- update `recoverability-postgres-v1` for the new structured RunHistory contract;
- replace `executor-postgres-qualification` with a real
  `wallet-nonce-postgres-qualification` task under `.#test-db`;
- rewrite `tests/integration/tests/cargo_metadata_contract.rs` so it requires the three executor
  packages to be absent, the new storage crate to be present in the storage layer, and generic
  Runtime/RunHistory crates to remain EVM-neutral; retain the existing `cargo-metadata-contract`
  leaf; and
- update the Nixfied graph and documentation so those checks are part of `.#ci`.

The database qualification must use managed PostgreSQL and restricted production roles. Migration
success or a zero-test package command is not qualification.

After the final code and hardening commits are complete, run:

```text
nix run .#model-check
nix run .#ci
git diff --check
```

Do not run `.#check`, `.#test`, and `.#test-db` immediately before `.#ci`; the final gate already
composes them.
