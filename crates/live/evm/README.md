# mfm-evm-live

Direct provider registration for the three balance-oriented EVM Read capabilities, the generic
anchored contract-call Read, and the development-only durable EVM transaction Effect. The balance
callbacks retain the existing `EvmProvider` contract. Anchored calls bind an `EvmTransactionRoute`;
transaction execution binds the complete route, authority epoch, and sender account.

`EvmAdapterLocator` is one bounded private HTTP(S) URL. It implements neither `Debug`, `Display`,
nor serialization. Non-HTTP schemes, fragments, and control characters are rejected.

`JsonRpcEvmProvider` implements both the existing observational `EvmProvider` and the separate
typed `EvmTransactionProvider`. It owns one endpoint URL, one 10-second per-request deadline, a
512 KiB response bound, exact JSON-RPC IDs, and no proxy, redirect, referer propagation, or automatic
retry. The transaction facet exposes only chain/genesis, pending nonce, receipt, canonical block,
and exact-raw submission operations. JSON values are discarded at ingress in favor of checked
private-field types.

The per-operation observation contract every `EvmProvider` owes the domain is on the `EvmProvider`
trait rustdoc. Its one trap: `confirm-balance-anchor` re-observes the committed block its intent
names and never the head. The domain compares that result to the anchor it pinned before the
balance reads, so a head read would report ordinary chain progression as a reorg and fail every
collection on a chain that produces blocks.

Existing balance reads retain their reviewed JSON-RPC-error/empty-call SafeFailure policy. The
transaction and anchored-call paths treat every JSON-RPC error, unexpected null, malformed field,
oversize body, and transport failure as Unavailable; receipt null alone means not yet mined.
Anchored block absence is SafeFailure, codeless target is Rejected, and replacement of the authored
block is authenticated IntegrityBlocked evidence. Local operation, route, binding, signer purpose,
public-key-derived sender, or retained-authority mismatch is Internal before authority/provider IO.

The pure codec maps the checked domain command into pinned `alloy-consensus` 1.6.1 `TxEip1559`
values. Alloy owns signed EIP-2718 encoding, exact decoding, transaction hashing, and CREATE-address
derivation. The adapter still rejects the wrong transaction type, non-exact or command-mismatched
retained bytes, invalid or high-S signatures, wrong hashes, recovered-key mismatches, and sender
mismatches. Keccak helpers expose only checked public hash/address results; private key custody
remains in `mfm-keystore`.

Transaction execution is caller-driven and loop-free. It loads authority first, reserves one
pending nonce, signs once through the captured immutable-purpose handle, retains exact raw bytes,
checks receipt before submission, and submits at most once per invocation. A matching submission
response returns normal Runtime `Pending` progress;
transport failure, a dropped acknowledgement, malformed ingress, or a mismatched returned hash is
`Unavailable`. Receipt settlement is inserted after one validated receipt and one matching
canonical block observation. Settled evidence is an authority fast path with no signer/provider
call. Dropped futures resume from the append-only reservation/preparation/settlement facts.
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
