# EVM Transactions

Status: unavailable product capability; qualification target for commit 6

The core product has no registered EVM mutation state, executor binding, entry point, CLI command,
or REST route. Portfolio assembly is read-only. Current app admission cannot certify or execute an
EVM write, and runtime has no EVM-specific mutation path.

Reusable EVM model, encoding, transport, and signing primitives may remain library foundations.
They grant no product admission, target-entry, or journal authority.

## Generic Effect Seam

The complete MFM effect protocol is domain-free:

```text
pure state request authorship
  -> StateTransitionCommitted(EffectRequested)
  -> ExternalAccessAuthorized(EnsureEffect)
  -> one affine call to a qualified keyed executor
  -> ExternalAccessObserved(executor frontier or terminal evidence)
  -> state terminal-evidence callback
  -> StateTransitionCommitted(EffectSettled)
```

`EffectRequested` fixes the original `StateFrame`, immutable typed request, request digest,
idempotency/effect identity, and exact executor binding. It is the MFM outbox and survives every
process.

The MFM journal owns no mutation delivery phases, nonce allocator, signer workflow, resource
ownership, rebroadcast policy, receipt poller, or destination recovery rule. Repeated
`drive_once` calls invoke the same keyed executor `ensure` contract. The executor either returns a
stronger bounded frontier/terminal proof or a reviewed pending/safe-failure result. Only the state
callback can interpret certified terminal evidence and settle the node.

## Executor Qualification Required

An EVM transaction can be registered only when one durable wallet or relayer implementation
qualifies all of:

- immutable request/effect identity;
- exact tenant and executor-deployment generation binding;
- non-rollback delivery ledger;
- permanent stale/sibling-writer exclusion;
- destination convergence after request, response, process, host, and database ambiguity;
- typed sender/nonce resource ownership across every writer that can use that wallet;
- deterministic transaction construction and signing ownership;
- safe rebroadcast or exact-hash observation semantics;
- restart, backup, restore, failover, and reorganization behavior;
- bounded cumulative delivery evidence and terminal tombstone;
- exact-attempt target receipt linkage; and
- complete no-secret retained evidence.

PostgreSQL persistence alone is insufficient. The MFM journal writer fence protects journal
lineage, not wallet ownership, nonce exclusivity, signer/envelope custody, destination convergence,
or executor generation.

Memory and file executor backends are conformance surfaces only and cannot enable product
mutation.

## Candidate EVM Request Contract

Commit 6 may introduce one immutable EIP-1559 request whose semantic identity includes:

- semantic network and non-zero chain id;
- expected sender;
- signer/executor binding references;
- deterministic signing profile;
- one closed direct `Create` or ordinary `Call` action;
- value, calldata/init code, and access list;
- checked fee and gas policy; and
- any exact prerequisite value refs.

The request must exclude mutable routing, endpoint, credential, pending nonce observation,
signature, raw signed envelope, provider body, keystore path, unlock path, password, private key,
and mnemonic.

Any future transaction construction must use one canonical Alloy path for the type-2 signing
digest, signed encoding, and transaction hash. Width conversions to Alloy's chain-id/nonce/gas and
fee representations must fail closed without truncation or fallback. A deterministic recoverable
low-s signing profile and expected sender must be verified before target entry.

These are qualification constraints, not current product behavior.

## Candidate Resource Contract

The executor—not MFM runtime—must own a typed resource stream for sender/nonce allocation. Its
allocation record binds:

- exact resource ownership;
- resource key;
- policy reference;
- policy-configuration reference; and
- immutable typed allocation state.

Restart refolds and revalidates the stream under the same policy/configuration pair before another
authorization. A local process mutex, MFM worker identity, database session lock, or journal row
cannot reserve a nonce against another wallet, relayer, operator, deployment, or process.

## Target Entry And Observation

The executor must commit one delivery authorization before returning affine
`TargetEntryAuthority`. The destination adapter consumes it exactly once and returns an affine
`TargetOperationReceipt`. Only that receipt can append the matching executor observation.

An inconclusive submit exchange never authorizes a different transaction. Recovery must converge
on the immutable request through the qualified wallet/relayer contract. Exact-hash observation,
rebroadcast, replacement, nonce reuse, and reorganization policy must be part of that executor's
certified equivalence and resource contract rather than runtime heuristics.

The executor terminal claim must identify one exact returned observation and terminal tombstone.
The MFM store admits its canonical objects through a later audited ensure observation and creates
the producer-bound `ValueRef` values. The executor cannot append `EffectSettled`.

## Receipt And Finality Requirements

If commit 6 admits receipt/finality evidence, its typed contract must validate:

- exact transaction hash and immutable public transaction fields;
- explicit success or revert status;
- coherent block, transaction, and log identities;
- no logs for a reverted top-level transaction;
- direct-create address derivation where applicable;
- exact canonical block placement;
- certified confirmation depth; and
- behavior under receipt disappearance, movement, and reorganization.

Finality level is state/executor contract data fixed before admission. A launch-time default,
provider claim, or runtime policy cannot change it.

## Secret Boundary

Signatures and raw signed transactions are bearer mutation material. They remain transient inside
the qualified wallet/relayer target boundary and are never:

- typed state values;
- executor request/result objects;
- journal records or retained objects;
- facts or public outputs;
- replay inputs;
- portable export members;
- diagnostics; or
- logs and fixtures.

Only reviewed public identities, immutable unsigned intent, exact hashes, typed receipts, and
closed safe failures may cross into retained evidence.

## Replay

Recorded-history verification checks the generic effect request, authorization/observation links,
executor frontier chain, terminal proof, and `EffectSettled` relationship without calling an
executor or EVM provider.

Exact reproduction may recompute pure request and settlement relations from retained public
evidence, but it never reconstructs a signature, opens a keystore, resolves a route, submits,
polls, or appends. Candidate comparison is equally capability-free.

## Registration Change

Commit 6 must add registration and tests as one reviewed vertical change only after every executor,
destination, resource, signing, restart, and reorganization gate passes. Failure leaves EVM
mutation unavailable. It must not restore a special runtime phase model, add a compatibility
submission path, or weaken the generic effect seam.
