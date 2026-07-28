# EVM Transactions

Status: qualified registered product capability

The product publishes `mfm.evm/submit-transaction@1`. Its one-state graph uses the generic
recoverable-effect runtime seam, one exact verified executor binding, the separately fenced
PostgreSQL executor ledger, typed sender/nonce allocation, guarded deterministic signing, and the
stateless exact-generation EVM transport. It adds no EVM-specific runtime phase, CLI signing
command, REST mutation shortcut, or second run-state authority.

The domain model, encoding, transport, and signing primitives remain reusable libraries. Only the
qualified product registration and deployment composition grant admission or live executor access;
the primitives alone grant neither target-entry nor journal authority.

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

## Qualified Executor Boundary

The registered wallet executor qualifies:

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

The production backend is `mfm-storage-executor-postgres`, opened only after the complete product
support graph is admitted and only with an `ExecutorWriterGenerationFence`. That fence is
independent of the MFM journal writer fence and must prove the exact executor binding, tenant,
ledger generation, database/schema lineage, and stale/sibling-writer exclusion. PostgreSQL
persistence alone still proves none of wallet ownership, nonce exclusivity, signer/envelope
custody, destination convergence, or non-rollback generation.

Memory and file executor backends are conformance surfaces only and cannot enable product
mutation.

## EVM Request Contract

The registered entry point resolves one immutable EIP-1559 request whose semantic identity
includes:

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

Transaction construction uses one canonical Alloy path for the type-2 signing
digest, signed encoding, and transaction hash. Width conversions to Alloy's chain-id/nonce/gas and
fee representations must fail closed without truncation or fallback. A deterministic recoverable
low-s signing profile and expected sender must be verified before target entry.

## Resource Contract

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

The account-sequence key is stable and non-secret:

```text
account         = "eip155-" + decimal_chain_id + "-" + lower_hex_sender_without_0x
resource_domain = "evm-wallet-domain-" + sha256(
    wallet_domain_ref.schema_id + ":" + wallet_domain_ref.content_digest
)
```

The first allocation uses the deployment-attested initial nonce. A sender cannot advance to the
next permanent allocation until the prior effect has immutable terminal evidence. Allocated
nonces are never reassigned to unrelated requests.

## Target Entry And Observation

The executor must commit one delivery authorization before returning affine
`TargetEntryAuthority`. The destination adapter consumes it exactly once and returns an affine
`TargetOperationReceipt`. Only that receipt can append the matching executor observation.

An inconclusive submit exchange never authorizes a different transaction. Recovery must converge
on the immutable request through the qualified wallet/relayer contract. Exact-hash observation,
rebroadcast, replacement, nonce reuse, and reorganization policy must be part of that executor's
certified equivalence and resource contract rather than runtime heuristics.

Guarded deterministic signing precedes broadcast authorization and derives one public
candidate-specific target-entry descriptor. The descriptor commits the exact operation kind,
unsigned candidate model, and transaction hash in the authorization append, but never the
signature or raw signed bytes. Recovery resolves that retained descriptor and looks up its hash
before considering rebroadcast. A policy-permitted rebroadcast re-signs behind the same generation
guard, verifies the newly derived hash and candidate reference against the committed descriptor,
and obtains a fresh authorization; signer unavailability is an operational retry without a
synthetic delivery attempt.

The executor terminal claim must identify one exact returned observation and terminal tombstone.
The MFM store admits its canonical objects through a later audited ensure observation and creates
the producer-bound `ValueRef` values. The executor cannot append `EffectSettled`.

## Receipt And Finality Contract

The typed receipt/finality contract validates:

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

## Registration And Deployment

Commit 6 adds the state, one-state operation, entry-point/profile registration, value and callback
contracts, executor-required leaf expansion, capability manifest member, qualified live executor,
and production composition as one vertical change. The app admits an immutable configured request
only when its embedded tenant equals the authorized tenant and its template target equals the
public selector.

Deployment must provide:

- the exact executor contract, deployment, resource ownership, and account-sequence binding;
- a dedicated executor PostgreSQL pool and independent writer-generation fence;
- the public wallet signer-binding reference and verified keystore signer binding;
- a signing-generation guard covering the executor generation, destination fence, and direct-sign
  exclusion; and
- an exact immutable EVM route generation.

The conformance suite covers same-key/different-request rejection, concurrent ensure, stale-plan
authorization, restart from durable bytes, response loss and signer-free hash recovery,
already-known classification, success and revert, pre-resolution reorganization, finite
rebroadcast/replacement equivalence, terminal retention, PostgreSQL fencing/refold, and
no-secret/no-bearer persistence. The product deliberately exposes no compatibility submission
path and no generic not-applied terminal outcome.
