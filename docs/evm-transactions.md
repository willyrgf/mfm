# EVM Transactions

MFM has one reusable EVM mutation state: `SubmitEvmTransactionState`. It is EIP-1559-only and one
state node always means one signed chain transaction. The state is registered for certification,
live execution, resume, and replay, but there is no standalone transaction operation, setup kind,
CLI route, or REST route.

## Actions and composition

`EvmTransactionAction` is closed:

- `Create { init_code, value }` is direct EOA contract creation.
- `Call { to, calldata, value }` is every ordinary invocation. Empty calldata is a native transfer.

A deployment followed by configuration is `Create -> Call -> Call`. Each arrow is a typed graph
dependency and each node has its own side-effect ledger. A contract-side multicall, smart-wallet
batch, factory deployment, CREATE2 deployment, swap, or flash-loan executor is one `Call` when it is
one atomic chain transaction. Operation crates own byte encoding and typed interpretation of logs;
the generic state owns no ABI JSON, function lookup, or multi-transaction workflow topology.

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
checked fee inputs, and gas estimate. The deterministic reducer fixes one unsigned envelope. The
adapter then binds the exact configured signer, signs once, verifies the profile and recovered
sender, and computes the expected hash from the exact signed EIP-2718 bytes.

The binding accepts `DeterministicSigningProvider`, not the unconstrained signing-provider trait.
The keystore binding registers implementation `mfm.signing.keystore.rfc6979.v1` and must report the
certified `secp256k1.rfc6979.recoverable.low_s.v1` profile before it can be called. A remote,
hardware, or other provider cannot bind to this transaction path merely because it can return a
valid secp256k1 signature; it must satisfy the byte-identical reconstruction contract or use a
different durable bearer-material design.

`EvmPreparedTransaction` retains:

- immutable authored intent;
- pending nonce, base/priority/maximum fee observations, and gas estimate;
- the exact unsigned EIP-1559 envelope and Alloy signing digest;
- the expected signed transaction hash;
- the sender/nonce-derived address for direct creation; and
- redacted network/chain/source/implementation session evidence.

It never retains signature scalars, a signed envelope, raw transaction bytes, endpoint, auth header,
provider body, keystore path, unlock path, password, private key, or mnemonic.

The ordinary path keeps the signed bearer in a bounded process-local one-shot cache between prepare
and submit. Submission consumes the same bytes. If the process is lost, recovery reconstructs the
retained envelope and asks the certified deterministic signer to regenerate it; the resulting hash
must equal prepared authority before any rebroadcast.

## Submission and recovery

`eth_sendRawTransaction` is accepted only when its returned hash equals the local hash. An exact-hash
transaction lookup must then match chain id, nonce, sender, destination/creation kind, value, input,
gas, fees, and access list. Persisted observation omits signatures and raw bytes.

Recovery first performs exact-hash lookup. If absent, it may rebroadcast only the identical prepared
envelope and look up the same hash again. Empty lookup, transport uncertainty, an external writer
occupying the nonce, or an inconclusive rebroadcast remains `SubmissionUnknown`. Standard JSON-RPC
cannot prove that a transaction was never submitted, so this path never emits
`NotSubmittedProven` and never chooses a replacement nonce under the same prepared invocation.

`SubmissionUnknown` retains only the prepared transaction hash and redacted checked-session
identity. A wrong provider submit hash or an exact-hash lookup whose public fields differ from the
prepared envelope records `SideEffectAmbiguous` with the closed
`mfm.evm.transaction_mismatch` code and the same redacted evidence. Replay validates both evidence
forms against prepared authority; neither contains a provider message, response body, signed
envelope, signature, endpoint, or credential.

## Receipt and finality

Receipts require explicit success or revert status and complete logs. Every log retains address,
topics, data, block number/hash, transaction hash/index, log index, and `removed`; removed or
identity-incoherent logs fail closed.

Only successful direct `Create` may carry `contract_address`, and it must equal the address derived
from fixed sender and nonce. Successful or reverted `Call`, plus reverted `Create`, must carry none.
A reverted receipt is a terminal external effect that consumed nonce and gas; the outcome reports
`Reverted(Create | Call)` and never exposes a created-contract handle.

For `Finalized { depth }`, verification fetches a fresh unchanged receipt, reads its block by number
and requires the exact retained hash, then reads a fresh head and checks `head - receipt + 1 >= depth`.
A disappeared or moved receipt, wrong canonical block, or shallow head remains pending. Confirmation
evidence retains the fresh receipt, canonical block, head, checked depth, and redacted session.

## Replay and secret boundary

The EVM transaction replay verifier decodes retained typed intent, prepared invocation, transaction,
receipt, and confirmation artifacts. It recomputes the unsigned plan, fee relation, signing digest,
CREATE address, lookup equality, receipt/log coherence, canonical block equality, and confirmation
depth. Replay never loads current runtime config, opens a route, calls JSON-RPC, constructs a signer,
or opens a keystore. Since signature material is deliberately absent, replay verifies the retained
expected-hash authority and its downstream relations; it does not claim an offline proof of an
omitted signature.
