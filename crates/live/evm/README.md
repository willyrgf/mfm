# mfm-evm-live

Direct provider registration for the three balance-oriented EVM Read capabilities, the generic
anchored contract-call Read, and the development-only durable EVM transaction Effect. The balance
callbacks and anchored calls share the typed `EvmReadProvider` boundary. Anchored calls bind an
`EvmTransactionRoute`;
transaction execution binds the complete route, authority epoch, and sender account.

`EvmAdapterLocator` is one bounded private HTTP(S) URL. It implements neither `Debug`, `Display`,
nor serialization. Non-HTTP schemes, fragments, and control characters are rejected.

`JsonRpcEvmProvider` implements both the observational `EvmReadProvider` and the separate
typed `EvmTransactionProvider`. It owns one endpoint URL, one 10-second per-request deadline, a
512 KiB response bound, exact JSON-RPC IDs, and no proxy, redirect, referer propagation, or automatic
retry. The transaction facet exposes only chain/genesis, pending nonce, receipt, canonical block,
and exact-raw submission operations. JSON values are discarded at ingress in favor of checked
private-field types.

The provider receives the checked broad or anchored intent plus Runtime's exact canonical intent
value ref and returns evidence carrying that same ref. No operation ID, serialized-intent byte
transport, adapter recanonicalization, or provider-side domain decode remains. The confirmation
subject re-observes the committed block its intent names and never the head.

Every JSON-RPC error, unexpected null, malformed field, oversize body, and transport failure is
Unavailable. An empty broad token-call result alone is SafeFailure where the balance contract
admits a missing token interface; receipt null alone means not yet mined.
Anchored block absence is SafeFailure, codeless target is Rejected, and replacement of the authored
block is authenticated IntegrityBlocked evidence. Local capability, route, binding, signer purpose,
public-key-derived sender, or retained-authority mismatch is Internal before authority/provider IO.

The pure codec maps the checked domain command into pinned `alloy-consensus` 1.6.1 `TxEip1559`
values. Alloy owns signed EIP-2718 encoding, exact decoding, transaction hashing, and CREATE-address
derivation. The adapter still rejects the wrong transaction type, non-exact or command-mismatched
retained bytes, invalid or high-S signatures, wrong hashes, recovered-key mismatches, and sender
mismatches. Keccak helpers expose only checked public hash/address results; private key custody
remains in `mfm-keystore`.

Transaction execution is caller-driven and loop-free. Registration validates the immutable
binding/epoch/purpose/public-key/sender composition once. Each invocation compares only the command
binding and exact Runtime-supplied command value ref, loads authority first, verifies the chain at
most once when not already settled, reserves one pending nonce, signs once through the captured
immutable-purpose handle, retains exact raw bytes, checks receipt before submission, and submits at
most once per invocation. Fresh Alloy transactions are not decoded again; retained wire is decoded
and fully compared before provider IO. A matching submission
response returns normal Runtime `Pending` progress;
transport failure, a dropped acknowledgement, malformed ingress, or a mismatched returned hash is
`Unavailable`. Receipt settlement is inserted after one validated receipt and one matching
canonical block observation. Settled evidence is an authority fast path with no signer/provider
call. A different concurrent Prepared or Settled candidate returns `Unavailable`; reload qualifies
the retained first winner before any retry. Dropped futures resume from the append-only
reservation/preparation/settlement facts.
Runtime supplies the exact qualified command value ref to the Effect callback; the adapter passes
that identity to authority qualification and does not recanonicalize the command.

This is the ONLY crate allowed to depend on `alloy-*`. The direct `alloy-consensus` and
`alloy-eips` dependencies are pinned to 1.6.1; the latter exposes the public EIP-2718 traits already
present in the consensus dependency graph. The confirmed normal tree adds Alloy's EIP, trie, RLP,
and transaction-macro support, while the selected feature tree activates no Alloy `k256`,
`secp256k1`, `c-kzg`, or `blst` backend. That bounded cost replaces the consensus-critical custom
unsigned/signed type-2 RLP implementation and its duplicate parser tests. The provider half of Alloy
would add unrelated transport and RPC surface, so `alloy-provider`, `alloy-network`,
`alloy-rpc-types`, and `alloy-signer` remain excluded. Verify with
`cargo tree -e features -p mfm-evm-live`.

The crate registers callbacks but owns no State registration, planner, binding wrapper, response
echo, secret custody, provider retry loop, or production finality configuration. Production
`ComposedRuntime`, CLI, REST, and configuration do not register these transaction or anchored-route
callbacks. Version 1 settlement is only the canonical-receipt policy of the pinned non-reorging Reth
development fixture.

Capability injection is not live registration: it is deterministic domain-owned Program topology
applied before Runtime sees the Program. This crate never invokes Operation or injection hooks.

Success envelopes require an explicit `result` field even for nullable receipt and block results.
An omitted field is Unavailable; explicit null alone represents absence. Malformed receipt ingress
retains Prepared without submission, and a later caller resumes the same exact bytes.
