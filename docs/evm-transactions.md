# Structured EVM Transaction Submission

Status: current production contract for `mfm.evm/submit-transaction@1`

## Material uncertainties

none

## Overview

EVM submission is an ordinary structured operation. The authored entry state expands purely into
visible `Pure`, `Read`, `Effect`, `Match`, child-fragment, and bounded recovery structure. Runtime
has no EVM-specific branch, and the wallet authority has no run scheduler.

```text
EvmSubmissionRequest
  -> validate the authorized nonce domain and derive authenticated intent
  -> read linearizable wallet status
  -> observe fresh pending nonce when reservation is needed
  -> reserve one stable intent/nonce
  -> build and attest a deterministic candidate
  -> activate the next candidate ordinal
  -> sign and broadcast that exact candidate once
  -> observe transaction, receipt, finalized head, and canonical inclusion
  -> reconcile or complete the wallet authority
  -> project EvmSubmissionOutput
```

Every step has a certified occurrence and exact failure handling. Polling, replacement,
reconciliation, and exhaustion are bounded by the admitted expansion.

The managed qualification pins the content-addressed certification identity of this production
entry program and certifies it again after reversing component registration order. Any authored,
expanded, or normalized representation change therefore requires deliberate identity regeneration
and review; the old root is not a compatibility input.

## Public failure contract

The sole typed operation failure is:

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

The expansion maps exact leaf failures into this sum. Possible entry, physical supersession,
malformed evidence, and integrity faults retain Runtime's non-domain cursor meanings and cannot be
converted into an EVM failure for liveness.

## Domain and activation

`WalletNonceDomain` is the pair of a qualified physical chain instance and sender. Chain identity
includes chain id, genesis, a never-reused instance namespace, and finalized fork anchor. Redundant
routes may name one instance; an independently operated clone cannot.

Qualified deployment infrastructure is the sole issuer of chain-instance and route-membership
authority. Before application assembly, an authenticated provider checks the complete public
routing catalog against its pinned chain-registry lineage and exact current head. The
resulting `QualifiedEvmRoutingCatalog` is opaque and non-serializable; catalog bytes alone cannot
construct it. Admission derives its routing policy from that value and requires the live route,
wallet activation, transaction intent, submission configuration, final admitted request, and
portfolio binding to carry the same full chain attestation and route membership. Numeric chain-id
equality never grants authority, and foreign binding combinations fail before admission.

Before first use, deployment permanently activates the domain against one store lineage. The
activation proof binds:

- exact chain instance, route membership, and sender;
- issuer namespace and current schema;
- store lineage and writer epoch;
- target-held public identity and current incarnation;
- qualified finalized sender-nonce floor;
- complete old writer/signer/relayer/operator/stale-deployment/direct-submit fencing; and
- terminal disposition for every prior effect and allocation.

Activation issuance is an append-only registry CAS. Normal request execution uses the immutable
proof and offline verification, with no registry query. Missing or incompatible proof requires a
new sender/domain.

## Stable intent

EVM configured-value history contains immutable transaction semantics only. It contains no tenant,
authenticated principal, or caller token. The public selector supplies a bounded caller token, and
the access policy supplies tenant and stable principal from the credential. Only after target-bound
authorization does the app construct the final request and derive its authenticated issuer.

`SubmissionIntentId` is derived from the wallet nonce domain, authenticated issuer identity, and
bounded caller token. It is never accepted as a free caller-supplied identifier. The caller token
is independent of the per-run `invocation_identity`: two invocations by the same authenticated
principal can converge on one permanent intent, while the same token used by two principals cannot.
Reservation, candidate, and completion keys exclude `run_id`, implementation references, and
physical generations.

One nonce domain permits one incomplete intent. The same intent can resolve and continue its
permanent progress from another run. A different intent returns `NonceDomainBusy` without
allocating a nonce. Changed canonical semantics under an existing id is an integrity conflict.

## Fresh pending-nonce rule

Every reservation attempt first commits a fresh
`eth_getTransactionCount(sender, "pending")` Read observation.

- Virgin lineage: pending must equal the activation proof's finalized sender-nonce floor, and that
  nonce is reserved.
- Later lineage: allocate `local_high_water + 1` only when pending is equal or behind.
- First-use mismatch or later provider-ahead: return `NonceLineageDiverged` without mutation.
- Exact retry after a permanent reservation exists: resolve the original proof; the new pending
  observation is creation-only evidence and cannot move the reserved nonce.

The authority transaction owns cross-run uniqueness. A producer-bound reservation value proves
within-run causality but is not the lock.

## Candidate family

The intent freezes a bounded mutation-equivalent candidate family. Each member has a stable ordinal
and deterministic unsigned identity. The authority retains a contiguous activated prefix; it
rejects skipped ordinals, foreign descriptors, duplicated hashes across ordinals, and replacement
without the exact predecessor/current-status/policy eligibility permit.

A later run reads the whole prefix, reproduces the current candidate, and observes every possible
winning hash. An older activated replacement may win and complete the one canonical result.

## Signing and broadcast

`AttestCandidateIdentity` is a Read that retains only the expected transaction hash. Qualified
signer generations implement one stable semantic signer: public key, address, algorithm, and
deterministic signing profile are fixed. A signer whose read consumes quota, approval, billing,
anti-replay state, rate-limit capacity, or other externally meaningful semantic state cannot
qualify as a Read. The guard and provider declare that immutable contract explicitly;
qualification, deployment handoff, live construction, and every signing callback recheck it before
guard or key access. Live attestation accepts only the opaque process-local qualified bearer, and
there is no raw-provider or Effect fallback. The local-keystore `v2` implementation opens and
decrypts without persisted audit mutation; deployments retain any `v1` physical release and append
a same-key-target `v2` successor.

`BroadcastExactCandidate` is an Effect. Its invoker deterministically reproduces and verifies the
signed envelope, submits that exact envelope once, and drops/zeroizes bearer bytes. Programs,
history, facts, outputs, logs, traces, exports, errors, and wallet storage never retain private
keys, signatures, or signed transactions.

If physical authority is revoked and non-entry is proved before broadcast, Runtime records
`SupersededBeforeEntry` and may authorize the next generation. If entry may have occurred,
`EntryUnknown` parks the occurrence; it cannot rebroadcast automatically.

## Observation and terminal convergence

Explicit Reads observe transaction presence, receipt, finalized head, and canonical inclusion.
The expanded program selects bounded reconciliation or terminal branches. Completion is valid only
for an active reservation and activated ordinal/hash. It atomically seals the activated prefix,
persists the canonical inclusion-block outcome closure, clears the active marker, and retains high
water.

`CompleteEvmNonceRequest` carries the complete bounded `TerminalWitnesses` value closure, not an
unresolved witness digest. The closure contains the exact committed transaction lookup and receipt,
finalized head, fresh inclusion-block lookup, selected terminal-assurance contract, and canonical
public projection. Domain construction cross-checks transaction and receipt hashes and blocks,
receipt status, finalized-head ordering, inclusion identity, assurance, and projection against the
run-independent `CanonicalTerminalOutcome`. The PostgreSQL authority independently repeats those
cross-links against the retained intent, reservation, and activated prefix before either resolving
or creating a completion.

Terminal idempotency compares the run-independent canonical result, not a later witness producer
reference. Exact replay and compatible later finalized-head witnesses resolve the original
completion; a conflicting canonical claim fails with integrity error. The original closure's
canonical content reference remains in `CompletedWalletNonce` as audit provenance. A completion
that becomes visible after one bounded run's last status snapshot remains available to a later run
rather than rewriting the earlier decision.

## PostgreSQL authority

`mfm-storage-evm-postgres` owns two separately credentialed SQL surfaces:

- activation-registry administration/public proof; and
- application wallet status/reserve/activate/complete.

The crate exposes typed authorities, not a generic pool. Its authenticated provider client also
qualifies complete routing catalogs against the pinned registry trust anchor; cloning that client
copies only endpoint configuration and public trust, while signing authority and target inventory
remain in the separate provider process. Each status read opens a transactionally consistent
snapshot through a sealed current target session. Each mutation uses a fresh, non-cloneable
transaction-bound permit. Permanent operation keys are resolved before and under the domain lock so
lost acknowledgements converge to their original proof.

The activation-registry admin, registry public, wallet application, run-history, owner, and test
roles cannot cross-write. Target/session issuance, registry/catalog authentication, and promotion
are supplied by qualified deployment infrastructure in a distinct process. Public request/reply
values are evidence only; they cannot mint authority. Normal status and mutation verify the
immutable activation proof offline and make zero chain-registry queries.
Normal status and reservation paths validate the maintained current projection, then load only the
exact frontier reservation, bounded candidate prefix, and optional completion; no lifetime
reservation scan occurs on normal paths. Full historical dense-prefix/count/max integrity is
checked during schema qualification/open-role validation. Immutable application-role history and
schema qualification preserve wallet append-only currentness, while externally retained checkpoints
preserve target and deployment lineage.
The managed qualification also compares the wire-level statement count for one retained status read
before and after 64 completed reservations and checks the analyzed unique-frontier index plan.

## Promotion

Same-domain promotion proceeds only in this order:

1. irrevocably fence and drain the old target and every sender path;
2. capture and verify the complete post-quiescence prefix;
3. hydrate and verify the still-closed replacement;
4. publish the next incarnation by exact-head registry CAS; and
5. open the replacement with a new target-held session.

Any missing fence, incomplete prefix, stale session, replayed permit, rollback, sibling issuer, or
incompatible schema/issuer namespace fails closed and requires a new sender/domain.

## Qualification evidence

The managed PostgreSQL qualification exercises real SQL, restricted roles, authenticated
chain-registry/catalog qualification, pinned lineage and head rejection, exact route-membership
binding, cross-chain rejection before admission, activation, first/later reservation rules,
idempotency, competing intents, candidate progression, completion, target copying, stale sessions,
permits, rollback, restart, and zero normal-execution registry queries. The structured submission
qualification additionally uses a loopback JSON-RPC server, deterministic signer, real wallet
authority, process restart, exact single broadcast, and later completion.
