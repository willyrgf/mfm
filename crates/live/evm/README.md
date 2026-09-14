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

Provider failures retain an `EvmOperationalError` with one closed kind and one boxed
`ProviderFailure`. Its exact wire has `kind` and `source`; the box is transparent. The source owns
the RPC method, stage, local checked rejection facts and Values' `DiagnosticEvidence`. Capture
retains exposed source messages, parser category/location, transport kind and OS kind/code.
Response status/code and RPC message/data remain outside ancestry; `data_json` preserves the
original numeric spelling as text. Unknown source types retain their exposed message and child
links. Whole-owner admission
applies the ordinary canonical/object limits without a separate diagnostic budget. Dependency text
is trusted diagnostic input; MFM does not deliberately add requests or credentials to that context.

`source_cycle: true` means exactly “traversal stopped on a repeated interface pointer.” The local
source walker compares complete `dyn Error` pointers with `std::ptr::eq`; it does not establish
concrete-object identity. An inline child may share its parent's data address, and one concrete
error can have different interface representations. The latter may produce repeated cause entries
before termination; no exact cyclic-object visit count is promised across compiler configurations.
There is no identity registry, message comparison or diagnostic budget.

Request/body deadlines are Timeout,
HTTP 429 is RateLimited, and other transport, JSON-RPC, unexpected-null, malformed-field and
oversize-body failures are Unavailable. An empty broad token-call result alone is SafeFailure where the balance contract
admits a missing token interface; receipt null alone means not yet mined.
Anchored block absence is SafeFailure, codeless target is Rejected, and replacement of the authored
block is authenticated IntegrityBlocked evidence. Local capability, route, binding, signer purpose,
public-key-derived sender, or retained-authority mismatch is Internal before authority/provider IO.

`fund_development_sender` uses the same bounded transport and exact decoder for unlocked-account
discovery and one funding submission, with the existing 16 KiB funding response bound. It has no
Program/custody authority and does not retry an ambiguous submission. The managed E2E owns its
funding amount and fees and retains build/provider causes through its helper.

The pure codec maps the checked domain command into pinned `alloy-consensus` 1.6.1 `TxEip1559`
values. Alloy owns signed EIP-2718 encoding, exact decoding, transaction hashing, and CREATE-address
derivation. The adapter still rejects the wrong transaction type, non-exact or command-mismatched
retained bytes, invalid or high-S signatures, wrong hashes, recovered-key mismatches, and sender
mismatches. Keccak helpers expose only checked public hash/address results; private key custody
remains in `mfm-keystore`.

`register_evm_transaction_adapters` registers reservation, preparation, and execution callbacks.
`register_evm_transaction_states::<C, R>` separately installs the four exact executable State
ABIs and injected failure map for an accumulated context and recipe; it binds no IO and performs no Program planning. Reservation captures binding,
custody, and provider; preparation captures binding, custody, and signer; execution captures no
signer. Reservation loads first, then observes pending nonce if absent. Preparation loads first,
signs only when necessary, and validates the immutable first winner returned by custody.
Execution decodes exact retained wire, recovers sender, checks receipt before submission, and
submits at most once per invocation. CPU work runs in immediately awaited blocking closures;
provider, signer, and custody IO stay outside them.

Transaction adapter errors retain provider causes inside `EvmTransactionOperationalError::Provider`;
custody and signer failures are AuthorityUnavailable and SignerUnavailable respectively. Local
invariants bypass recovery classification and carry no external detail.

A matching submission returns Pending. Transport failures, missing acknowledgements, malformed
ingress, or mismatched returned hashes are Unavailable. A validated receipt and matching canonical
block produce settlement evidence for Journal; custody stores no settlement. Cold terminal reads
need no signer or provider. External nonce advances are accepted for fresh reservations; displaced
old transactions remain unresolved without automatic renonce or conflict detection.

This is the ONLY crate allowed to depend on `alloy-*`. The direct `alloy-consensus` and
`alloy-eips` dependencies are pinned to 1.6.1; the latter exposes the public EIP-2718 traits already
present in the consensus dependency graph. The confirmed normal tree adds Alloy's EIP, trie, RLP,
and transaction-macro support, while the selected feature tree activates no Alloy `k256`,
`secp256k1`, `c-kzg`, or `blst` backend. That bounded cost replaces the consensus-critical custom
unsigned/signed type-2 RLP implementation and its duplicate parser tests. The provider half of Alloy
would add unrelated transport and RPC surface, so `alloy-provider`, `alloy-network`,
`alloy-rpc-types`, and `alloy-signer` remain excluded. Verify with
`cargo tree -e features -p mfm-evm-live`.

The crate registers callbacks and the reusable transaction State family, and owns no planner,
binding wrapper, response echo, secret custody, provider retry loop, or production finality configuration. Production
`ComposedRuntime`, CLI, REST, and configuration do not register these transaction or anchored-route
callbacks. Settlement is only the canonical-receipt policy of the pinned non-reorging Reth
development fixture.

Capability injection is not live registration: it is deterministic domain-owned Program topology
applied before Runtime sees the Program. This crate never invokes Operation or injection hooks.

Success envelopes require an explicit `result` field even for nullable receipt and block results.
An omitted field is Unavailable; explicit null alone represents absence. Malformed receipt ingress
retains Prepared without submission, and a later caller resumes the same exact bytes.

Authority unavailable evidence moves unchanged into EvmTransactionOperationalError v3; internal
InvocationDiagnostic moves unchanged into AdapterError::Invariant. The v3 owner also carries signer
cause data. Existing SigningError::Invalid/Failed produce only their actual kind and sign/signer
context, without invented deeper causes. SigningError::SignFailed moves its executing keystore
operation/stage/cause evidence unchanged into SignerUnavailable, classified Retryable.
The new owner schema changes dependent assembly contracts. This implementation performs no deployed
assembly replacement; v2 runs require an explicit handling decision before rollout, without rewriting
acknowledged history or adding a dual decoder.
