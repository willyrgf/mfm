# mfm-adapters-portfolio

Executable runner bindings for certified portfolio states.

## Live path

For each EVM network, the adapter validates the certified network/chain binding and asynchronously
binds one checked `EvmReadSession`. It resolves latest once, executes every balance and deduplicated
token-metadata call against that exact block hash with EIP-1898 `requireCanonical: true`, then
reads the original block number once to prove the hash is still canonical. Metadata and balance
reads are concurrent with a hard limit of 16 tasks. The state-owned reducer admits the evidence;
the managed-write runner then records every `evm.balance_snapshot` fact and its checked collection
receipt in one output commit.

`SelectHoldings` consumes the typed Bitcoin and EVM receipt vectors directly. The adapter submits
all receipt-derived requests in one fact-index batch, requires one shared snapshot frontier,
hydrates each returned response artifact, and records the state-produced query evidence. Portfolio
assembly therefore receives only store-reread and identity-reverified observations; an all-EVM
portfolio follows exactly the same path.

## Replay path

`verify_portfolio_replay` uses only certified configs, the append-only stream, and retained
artifacts. It invokes the same EVM collection reducer, recomputes every published fact and checked
receipt, verifies exact same-attempt `FactRecorded` coverage, and proves `SelectHoldings` consumed
the published receipt vector. BTC/EVM selection is recomputed from retained fact-query evidence,
and the pure snapshot/report outputs are compared byte-for-byte.

This crate binds capabilities and records evidence. It does not create workflow topology, select
runtime routes, implement JSON-RPC, or own state semantics.
