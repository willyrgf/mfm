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
assurance, and evidence bounds. The app admission path, executor, and JSON-RPC target share one
live-owned `Arc`; callers cannot provide parallel route, signer, policy, or safe-failure
descriptors.

No Bitcoin state, replay reducer, or aggregate reader is registered.
