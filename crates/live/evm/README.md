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
qualification artifacts verify the executable, shared 12-component
qualification, and exact adapter semantic/callback/implementation tuple. The
qualified factory borrows those artifacts and returns a closed six-entry typed
dispatch table for the sole program registry. All entries share one binding,
component implementation, routing catalog, and aggregate adapter. Qualification
also proves the safe classifier and failure contracts, reviewed source scope,
the routing catalog, and every immutable generation before registration.

No transaction/effect state, Bitcoin state, replay reducer, aggregate reader,
or mutation lifecycle is registered.
