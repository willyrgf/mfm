# EVM Transactions

MFM has one reusable EVM mutation state: `SubmitEvmTransactionState`. It is EIP-1559-only and one
state node always means one signed chain transaction. The state is registered for certification,
live execution, resume, and replay, but there is no standalone transaction operation, setup kind,
CLI route, or REST route. It is one of the four state kinds in the
[current EVM inventory](architecture.md#current-evm-inventory); registration does not add app
discovery.

## Actions and composition

`EvmTransactionAction` is closed:

- `Create { init_code, value }` is direct EOA contract creation.
- `Call { to, calldata, value }` is every ordinary invocation. Empty calldata is a native transfer.

A deployment followed by configuration is `Create -> Call -> Call`. Each arrow is a typed graph
dependency and each node has its own side-effect ledger. A contract-side multicall, smart-wallet
batch, factory deployment, CREATE2 deployment, swap, or flash-loan executor is one `Call` when it is
one atomic chain transaction. Operation crates own byte encoding and typed interpretation of logs;
the generic state owns no ABI JSON, function lookup, or multi-transaction workflow topology.

Exact validation composes independently at the receipt anchor. Both `Create -> Validate` and
`Create -> Call -> Call -> Validate` are covered end to end, including evidence-only replay; no
fixed deployment lifecycle or validation-only mutation path exists.

## Authored authority

`EvmTransactionConfig` fixes the semantic network and chain, expected sender, signer reference,
deterministic RFC 6979 recoverable low-s profile, access list, and the only admitted policies:

- fee: `base_fee * 2 + priority_fee`, with checked U256 arithmetic;
- gas: the exact checked `eth_estimateGas` result.

The action supplies bounded canonical init code/calldata and canonical decimal U256 value. The full
`EvmTransactionIntent` is also the kernel idempotency input. It intentionally excludes pending
nonce, fee observations, gas estimate, RPC source, and all signer output.

Every caller must plan the side effect with `evm_sender_lane_resource_claim()`. The runner resolves
the typed lane `(network_id, chain_id, expected_sender)` before preparation. This prevents two MFM
runs using the same store lane concurrently; it does not reserve the nonce against a browser wallet,
another MFM deployment, an operator, or any other process.

## Preparation and signing

After the lane claim commits, one source-bound `EvmTransactionSession` reads the pending nonce,
then the checked fee inputs. Intent and those observations are admitted once as the complete
pre-gas type-2 transaction description: chain id, nonce, sender, destination/create kind, value,
input, access list, maximum fee, and priority fee. Nonce and fee widths are checked against Alloy's
exact EIP-1559 representation before `eth_estimateGas` is called. The transport sends that full
object with type `0x2` against the `pending` block context. Adding the exact returned gas limit to
the same admitted description produces the unsigned envelope; no transaction field is reconstructed
from intent for signing. The adapter then binds the exact configured signer, signs once, verifies
the profile and recovered sender, and computes the expected hash from the exact signed EIP-2718
bytes.

The binding accepts `DeterministicSigningProvider`, not the unconstrained signing-provider trait.
The identity-bearing binder owns implementation `mfm.signing.keystore.rfc6979.v1`; transaction
capability registration derives its implementation identity from that binder rather than accepting
an unrelated label. A bound provider must report the same implementation id before any signature
request and must report the certified `secp256k1.rfc6979.recoverable.low_s.v1` profile before it can
sign. A remote, hardware, or other provider cannot bind to this transaction path merely because it
can return a valid secp256k1 signature; it must satisfy the byte-identical reconstruction contract
or use a different durable bearer-material design.

`EvmPreparedTransaction` retains:

- immutable authored intent;
- pending nonce, base/priority/maximum fee observations, and gas estimate;
- the exact unsigned EIP-1559 envelope and Alloy signing digest;
- the expected signed transaction hash;
- the sender/nonce-derived address for direct creation; and
- the canonical redacted network/chain/source/implementation `EvmSessionEvidence`.

It never retains signature scalars, a signed envelope, raw transaction bytes, endpoint, auth header,
provider body, keystore path, unlock path, password, private key, or mnemonic.

The ordinary path reserves bounded process-local cache capacity before nonce, fee, gas, or signer
access. Preparation captures that reservation and the signed bearer in a non-cloneable output
settlement. Only a durable `SideEffectInvocationPrepared` append promotes it to a fresh cache entry;
idempotent, admission-blocked, stale, failed, or dropped outputs destroy it. Saturation blocks the
attempt without evicting or deduplicating an active envelope. Submission leases those exact bytes
and marks them uncertain before the submission call. An uncertain lease is never submitted again:
the adapter performs exact-hash lookup and either records observation/ambiguity or retains only
uncertainty. Once durable `SubmissionUnknown` exists, recovery discards any remaining transient
envelope and is lookup-only. Runtime route reads, keystore reads, unlock/KDF work, and signer
validation execute on a blocking worker rather than an async runtime worker.

## Submission and recovery

`eth_sendRawTransaction` is accepted only when its returned hash equals the local prepared hash.
That acknowledgement and immutable prepared authority produce the persisted public submission
evidence directly; no immediate visibility lookup is required. If the call returns an error, the
adapter performs exact-hash lookup. A matching transaction must agree on chain id, nonce, sender,
destination/creation kind, value, input, gas, fees, and access list. A mismatched transaction or
wrong submit hash becomes ambiguous. Missing or unavailable lookup after the call becomes
`SubmissionUnknown`, never a terminal pre-submission failure.

Recovery of `SubmissionUnknown` performs exact-hash lookup only. Matching observation completes the
submission boundary, mismatch becomes ambiguity, missing remains unknown, and provider failure
blocks. Recovery never signs, submits, chooses another nonce, or treats unavailable lookup as
absence evidence. Standard JSON-RPC cannot prove that a transaction was never submitted, so this
path never emits `NotSubmittedProven`.

`SubmissionUnknown` retains only the prepared transaction hash and redacted checked-session
identity. A wrong provider submit hash or an exact-hash lookup whose public fields differ from the
prepared envelope records `SideEffectAmbiguous` with the closed
`mfm.evm.transaction_mismatch` code and the same redacted evidence. Replay validates both evidence
forms against prepared authority; neither contains a provider message, response body, signed
envelope, signature, endpoint, or credential.

## Receipt and finality

Receipts require explicit success or revert status. Every successful-receipt log retains address,
ordered topics, data, block number/hash, transaction hash/index, log index, and `removed`; removed or
identity-incoherent logs fail closed. Operations inspect every field through immutable typed
accessors and can identify/decode events with Alloy without serializing or reparsing the receipt.
Receipt transaction index, sender, destination, gas quantities, and session provenance are exposed
through the same read-only surface. A reverted top-level transaction cannot emit durable logs, so a
reverted receipt containing any log is rejected at live capability conversion and independently by
persisted/replay validation; logs are never stripped or normalized away.

Only successful direct `Create` may carry `contract_address`, and it must equal the address derived
from fixed sender and nonce. Successful or reverted `Call`, plus reverted `Create`, must carry none.
A reverted receipt with no logs is a terminal external effect that consumed nonce and gas; the
outcome reports `Reverted(Create | Call)` and never exposes a created-contract handle.

For `Finalized { depth }`, verification fetches a fresh unchanged receipt, reads its block by number
and requires the exact retained hash, then reads a fresh head and checks `head - receipt + 1 >= depth`.
A disappeared or moved receipt, wrong canonical block, or shallow head remains pending. Confirmation
evidence retains the fresh receipt, canonical block, head, checked depth, and redacted session.
Transaction, validation, collection, and portfolio evidence use the same capability-owned
`EvmBlockAnchor`, which persists the full U256 block number as canonical decimal and never narrows
it to u64.

Provider unavailability while reading the receipt, canonical block, or head is also an operational
block. Resume re-observes from the durable submission phase and never executes the submit node or
broadcast again. Malformed retained evidence, a regenerated signed-hash mismatch, or a certified
authority violation remains terminal validation failure.

## Replay and secret boundary

The EVM transaction replay verifier reconstructs the submit state from certified config, input, and
context. It reauthors the intent, runtime idempotency key, and sender-lane resource key, then decodes
the retained prepared invocation, transaction, receipt, and confirmation artifacts and invokes the
same state output reducer used live. It also recomputes the unsigned plan, fee relation, signing
digest, CREATE address, lookup equality, receipt/log coherence, canonical block equality, and
confirmation depth. Replay never loads current runtime config, opens a route, calls JSON-RPC,
constructs a signer, or opens a keystore. Since signature material is deliberately absent, replay
verifies the retained expected-hash authority and its downstream relations; it does not claim an
offline proof of an omitted signature.
