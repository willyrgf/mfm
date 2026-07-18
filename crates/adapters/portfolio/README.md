# mfm-adapters-portfolio

Executable runner bindings for certified portfolio states.

## Live path

For each EVM network, the adapter validates the certified network/chain binding and asynchronously
binds one checked `EvmReadSession`. It resolves latest once, executes every balance and deduplicated
token-metadata call against that exact block hash with EIP-1898 `requireCanonical: true`, then
reads the original block number once to prove the hash is still canonical. Metadata and balance
reads are concurrent with a hard limit of 16 tasks. The state-owned reducer admits the evidence;
the managed-write runner then records every unified fact and the direct network snapshot in one
output commit.

Bitcoin remains fact-backed. `SelectHoldings` consumes the exact Bitcoin receipt, performs the
bounded query/hydration/identity checks, and records query evidence. When the request batch is
empty, as for an all-EVM portfolio, the adapter does not call the fact index.

## Replay path

`verify_portfolio_replay` uses only certified configs, the append-only stream, and retained
artifacts. It invokes the same EVM collection reducer, recomputes every published fact and direct
snapshot, verifies exact same-attempt `FactRecorded` batch coverage, and proves assembly consumed
those snapshots unchanged. Bitcoin selection is recomputed from retained fact-query evidence, and
the pure snapshot/report outputs are compared byte-for-byte.

This crate binds capabilities and records evidence. It does not create workflow topology, select
runtime routes, implement JSON-RPC, or own state semantics.
