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

- immutable request/effect identity and the complete wallet policy before admission or allocation;
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

One live-owned `EvmWalletRequestQualification` is the sole cross-field predicate. It is built from
the sealed transport catalog, verified executor binding, product object-evidence contract, derived
guarded-signer descriptor, selected route generation, and initial-nonce descriptor. Construction
derives and binds the complete ordered route-generation-to-chain map and verifies the executor's
exact request/result contracts, wallet leaf expansion, target callback, resource domain/fence,
safe-failure contract, and evidence-contract closure. The canonical proof, content reference, and
debug form contain no endpoint, authorization, or transport internals. The live qualification
separately retains a private clone sharing the exact transport runtime and route catalog; the
executor obtains its transport from that qualification and has no injection argument. The
application admission path and executor share the same `Arc`; neither accepts an independently
supplied transport, signer reference, resource-policy pair, route descriptor, or duplicate
validator.

Valid replacement schedules and convergence plans remain request-owned semantic fields. They may
vary between admitted requests while fitting the exact qualified evidence bounds; the deployment
qualification deliberately does not pin either field to one deployment value.

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

- exact tenant, wallet domain, selected catalog-member route generation, and its non-zero chain
  id;
- expected canonical non-zero sender and the derived guarded-signer descriptor reference;
- exact nonce-policy reference, deployment-attested initial nonce, and complete initial-nonce
  descriptor reference;
- exact already-known classifier, finalized-tag finality policy, and terminal assurance policy;
- exact executor evidence bounds;
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
The signed EIP-2718 envelope is admitted at no more than 512 KiB immediately after Alloy encoding
and before hashing, then checked against the same bound again when the exact RPC body is assembled.

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

The first allocation uses the deployment-attested initial nonce. Its content-addressed descriptor
binds the nonce, public source-attestation reference, wallet domain, chain, sender, and durable
generation. The account-sequence policy reference is derived from the canonical EVM nonce policy;
deployment cannot supply either half of the policy/configuration pair independently. A sender
cannot advance to the next permanent allocation until the prior effect has immutable terminal
evidence. Allocated nonces are never reassigned to unrelated requests.

## Target Entry And Observation

The executor must commit one delivery authorization before returning affine
`TargetEntryAuthority`. The destination adapter consumes it exactly once and returns an affine
`TargetOperationReceipt`. Only that receipt can append the matching executor observation.

An inconclusive submit exchange never authorizes a different transaction. Recovery must converge
on the immutable request through the qualified wallet/relayer contract. Exact-hash observation,
rebroadcast, replacement, nonce reuse, and reorganization policy must be part of that executor's
certified equivalence and resource contract rather than runtime heuristics.

The only public live execution seam is `EvmWalletExecutor<Store>` with the concrete exact-generation
`EvmJsonRpcTransport`. The five wallet operations are private typed transport methods. There is no
generic wallet RPC client, arbitrary method/parameter call, raw JSON response, or public target
wrapper that can bypass typed decoding or qualification.

Guarded deterministic signing precedes broadcast authorization and derives one public
candidate-specific target-entry descriptor. The descriptor commits the exact operation kind,
unsigned candidate model, and transaction hash in the authorization append, but never the
signature or raw signed bytes. Recovery resolves that retained descriptor and looks up its hash
before considering rebroadcast. A policy-permitted rebroadcast re-signs behind the same generation
guard, verifies the newly derived hash and candidate reference against the committed descriptor,
and obtains a fresh authorization; signer unavailability is an operational retry without a
synthetic delivery attempt.

Before any signer call, RPC exchange, terminal append, pending return, terminal return, or
authorization/terminalization conflict return, the executor reloads one complete verified wallet
history. It reconstructs the deterministic plan for every retained attempt and validates the exact
candidate descriptor and result transition. Terminal evidence is accepted only when the complete
request, prior-result references, candidate lineage, transaction, receipt, finalized head,
canonical inclusion, outcome, executor generation, generation fence, and assurance tuple equal the
history-derived terminal. A retained tombstone must then identify that exact operation, outcome,
attempt, returned result, and returned observation. The history fold follows immutable append order
and freezes the expected plan at each authorization from authorization-ordered results whose
observations are already present. Later observations validate only against their frozen plan and do
not retroactively alter a later authorization. The first observed valid terminal is selected;
subsequent observations, including a legal post-tombstone observation, are audit-only after
validation. Any mismatch on restart fails without signing, RPC, or append; an exact restored
tombstone returns without any of those actions.

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
- logs or durable fixtures.

Loopback transport tests use only runtime-generated ephemeral sentinels, zeroizing socket/body
owners, borrowed JSON projections, and direct decode into zeroizing signed-byte storage. Test state
may retain public methods and transaction hashes, never authorization or raw signed bytes.

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

The current transaction entry point includes the state, one-state operation, entry-point/profile
registration, value and callback contracts, executor-required leaf expansion, capability manifest
member, qualified live executor, and production composition as one vertical path. The app admits
an immutable configured request only when its embedded tenant equals the authorized tenant and its
template target equals the public selector.

Deployment must provide:

- the exact executor contract, deployment, and resource ownership;
- a dedicated executor PostgreSQL pool and independent writer-generation fence;
- the selected immutable EVM route-generation reference and complete initial-nonce descriptor;
- the verified keystore signer binding, from which the public signer-descriptor reference is
  derived;
- a signing-generation guard covering the executor generation, destination fence, and direct-sign
  exclusion.

Bootstrap rejects any route/catalog, chain, signer, sender, generation, fence, wallet-domain,
nonce, classifier, finality, assurance, evidence-bound, executor-semantic, or object-evidence
mismatch before support admission. Configured request admission repeats the exact sealed
predicate before certification and journal append. Executor entry repeats it before effect binding
and nonce allocation; a rejected request creates no signer call, RPC call, effect record, or
resource allocation.

The conformance suite covers same-key/different-request rejection, concurrent ensure, stale-plan
authorization, restart from durable bytes, response loss and signer-free hash recovery,
already-known classification, success and revert, pre-resolution reorganization, finite
rebroadcast/replacement equivalence, terminal retention, PostgreSQL fencing/refold, and
no-secret/no-bearer persistence. The product deliberately exposes no compatibility submission
path and no generic not-applied terminal outcome.
