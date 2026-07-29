# mfm-evm-live

This crate owns the exact-generation EVM JSON-RPC transport and runtime
bindings for six independently audited read operations:

- `eth_chain_id`
- `eth_get_block_by_number_latest`
- `eth_call_erc20_decimals`
- `eth_get_balance`
- `eth_call_erc20_balance_of`
- `eth_get_block_by_number_confirm`

Every typed transport method performs zero or one HTTP exchange. Immutable
generation lookup and source checks happen locally; redirects, retries,
failover, current-route aliases, batching, arbitrary method calls, and
aggregate reductions are absent. Fan-out reads use the exact EIP-1898 block
hash with `requireCanonical: true`, and final confirmation resolves the initial
block number to a hash.

The transport has one private 11-operation allowlist: the six reads above and the five wallet
operations below. Requests are encoded directly into exact-capacity zeroizing byte owners; there is
no generic method/parameter or `serde_json::Value` request path. Authorization is consumed into a
sensitive owner-backed header, raw broadcasts are limited to 512 KiB, and every response is read
once into a preallocated zeroizing 1 MiB buffer. Only exact HTTP `200` is accepted.

Response decoding retains borrowed raw JSON ranges. It closes the envelope to exact JSON-RPC
version/id/result-or-error fields, rejects semantic duplicate keys including escaped aliases, and
lexically bounds ignored projections to depth 64 and 4096 items per container. Read results have an
additional 128 KiB semantic limit. Provider error messages are decoded only inside a 16 KiB
zeroizing owner and are discarded after exact already-known classification; no provider text or
data reaches a failure value.

After the app admits its one complete support graph, the sealed EVM
qualification artifacts verify the executable, shared 14-component
qualification, and exact read-adapter plus wallet-executor
semantic/callback/implementation tuples. The
qualified factory borrows those artifacts and returns a closed six-entry typed
dispatch table for the sole program registry. All entries share one binding,
component implementation, routing catalog, and aggregate adapter. Qualification
also proves the safe classifier and failure contracts, reviewed source scope,
the routing catalog, and every immutable generation before registration.

The durable wallet executor uses the same transport for five one-exchange target operations:

- `eth_sendRawTransaction`
- `eth_getTransactionByHash`
- `eth_getTransactionReceipt`
- `eth_getBlockByNumber("finalized", false)`
- `eth_getBlockByNumber(inclusion_number, false)`

Every broadcast is signed transiently behind the exact generation guard only after sender/nonce
allocation and immediately before durable target authorization. Hash, receipt, finality, and
canonical-inclusion recovery use retained public candidate descriptors without reopening the
signer. Raw signed bytes are zeroized and never enter executor evidence.

`EvmWalletExecutor<Store>` is the only public wallet execution type. It obtains the concrete
`EvmJsonRpcTransport` only from the sealed wallet qualification; its constructor has no independent
transport argument. Raw wallet RPC clients, responses, errors, target wrappers, and retained
target-entry descriptors are private implementation details.

After request qualification and permanent nonce allocation, every initial, recovered,
post-exchange, authorization/terminalization-conflict, pending, and terminal decision passes
through one wallet-history fold. The fold reconstructs the deterministic plan at each attempt,
validates the exact descriptor and typed result, derives terminal evidence only from that history,
and requires any tombstone to name the exact operation, outcome, attempt, returned result, and
returned observation. It reads immutable records in append order, freezes each plan when its
authorization appears from the authorization-ordered evidence observed by then, and validates a
later observation at its physical position without retroactively changing already authorized
plans. Authorization order remains the logical result-reference order. The first valid terminal
observation selects terminalization; every later observation, including one after the tombstone, is
validation-only audit evidence. Invalid restored history performs no signing, RPC, target
authorization/observation, or terminal append; replaying a valid terminal tombstone is likewise
signer-, RPC-, and write-free.

`EvmWalletRequestQualification` is the single sealed pre-admission and pre-allocation wallet
predicate. It derives the complete ordered route-generation/chain map from the actual transport,
closes the verified executor semantics and object-evidence contract, and binds the guarded-signer
descriptor, wallet domain, generation fence, nonce configuration, classifier, finality,
assurance, and evidence bounds. Its canonical proof, content reference, and debug representation
are secret-free and exclude endpoints, authorization, and transport internals. The live
qualification separately retains a private clone sharing the exact transport runtime and route
catalog; clone equality requires that same instance. The app admission path, executor, and
JSON-RPC target share one live-owned `Arc`; callers cannot provide parallel transport, route,
signer, policy, or safe-failure descriptors.

No Bitcoin state, replay reducer, or aggregate reader is registered.
